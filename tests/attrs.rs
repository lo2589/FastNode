mod common;

use common::{eq, ids, memory, node, nodes, p};
use fastnode::{NewNode, Predicate, Query};
use serde_json::{Value, json};

#[test]
fn any_depth_wildcard_and_value_anywhere() {
    let mut db = memory();
    let first = db
        .create(NewNode::new(
            "person",
            "nested",
            json!({
                "name":["a","b","c","d"],
                "g":{"k":{"d":["f","g","b","e"]}},
                "jobs":[{"company":"A","salary":10,"tags":["x"]},{"company":"B","salary":20,"teams":[{"lead":"L"}]}],
                "*":{"literal":1}
            }),
        ))
        .unwrap();
    let second = db
        .create(NewNode::new(
            "b",
            "second",
            json!({"jobs":[{"company":"B"}],"company":"A"}),
        ))
        .unwrap();
    assert_eq!(ids(&mut db, eq("/name", json!("b"))), vec![first]);
    assert_eq!(ids(&mut db, eq("/g/k/d", json!("b"))), vec![first]);
    assert_eq!(ids(&mut db, eq("/g/k/d/2", json!("b"))), vec![first]);
    assert_eq!(ids(&mut db, json!({"op":"any","value":"b"})), vec![first]);
    assert_eq!(
        ids(&mut db, json!({"op":"any","value":"A"})),
        vec![first, second]
    );
    assert!(ids(&mut db, json!({"op":"any","value":"person"})).is_empty());
    assert_eq!(ids(&mut db, eq("/jobs/1/company", json!("B"))), vec![first]);
    assert_eq!(
        ids(&mut db, eq("/jobs/*/company", json!("B"))),
        vec![first, second]
    );
    assert_eq!(ids(&mut db, eq("/jobs/*/company", json!("A"))), vec![first]);
    assert_eq!(ids(&mut db, eq("/jobs/*/tags", json!("x"))), vec![first]);
    assert_eq!(
        ids(&mut db, eq("/jobs/*/teams/*/lead", json!("L"))),
        vec![first]
    );
    let salary = json!({"op":"range","field":"/jobs/*/salary","gte":15});
    assert_eq!(ids(&mut db, salary.clone()), vec![first]);
    assert_eq!(
        ids(&mut db, json!({"op":"exists","field":"/jobs/*/teams"})),
        vec![first]
    );
    assert_eq!(ids(&mut db, eq("/~2/literal", json!(1))), vec![first]);
    assert!(db.select(&p(eq("/x~2y", json!(1)))).is_err());
    db.patch(
        first,
        json!({"attrs":{"jobs":[{"company":"A","salary":10}]}}),
    )
    .unwrap();
    assert_eq!(
        ids(&mut db, eq("/jobs/*/company", json!("B"))),
        vec![second]
    );
    assert!(ids(&mut db, salary).is_empty());
    assert!(ids(&mut db, eq("/jobs/*/teams/*/lead", json!("L"))).is_empty());
    db.delete(first).unwrap();
    db.delete(second).unwrap();
    assert_eq!(db.stats().unwrap().bitmap_blocks, 0);
}

#[test]
fn scalar_types_null_missing_and_empty_collections() {
    let mut db = memory();
    db.create_many(nodes(vec![
        json!({"x":1}),
        json!({"x":"1"}),
        json!({"x":true}),
        json!({"x":null}),
        json!({}),
        json!({"x":[]}),
        json!({"x":{}}),
    ]))
    .unwrap();
    assert_eq!(ids(&mut db, eq("/x", json!(1.0))), vec![1]);
    assert_eq!(ids(&mut db, eq("/x", json!("1"))), vec![2]);
    assert_eq!(ids(&mut db, eq("/x", json!(true))), vec![3]);
    assert_eq!(ids(&mut db, eq("/x", Value::Null)), vec![4]);
    assert_eq!(ids(&mut db, json!({"op":"any","value":"1"})), vec![2]);
    assert_eq!(
        ids(&mut db, json!({"op":"exists","field":"/x"})),
        vec![1, 2, 3, 4, 6, 7]
    );
    let not_one = json!({"op":"not","arg":eq("/x",json!(1))});
    assert_eq!(ids(&mut db, not_one), vec![2, 3, 4, 5, 6, 7]);
    assert_eq!(
        ids(&mut db, json!({"op":"and","args":[]})),
        vec![1, 2, 3, 4, 5, 6, 7]
    );
    assert!(ids(&mut db, json!({"op":"or","args":[]})).is_empty());
    assert!(ids(&mut db, json!({"op":"in","field":"/x","values":[]})).is_empty());
}

#[test]
fn exact_decimal_order_and_equivalence() {
    let mut db = memory();
    let values = [
        "-1e100",
        "-18446744073709551615",
        "-1.2",
        "-1.11",
        "-1.1",
        "-1",
        "-0.001",
        "-1e-20",
        "0",
        "1e-20",
        "0.001",
        "1",
        "1.01",
        "1.1",
        "1.11",
        "1.2",
        "9007199254740992",
        "9007199254740993",
        "18446744073709551615",
        "1e100",
    ];
    for text in values {
        let n: Value = serde_json::from_str(text).unwrap();
        db.create(node(json!({"n":n}))).unwrap();
    }
    for (i, text) in values.iter().enumerate() {
        let bound: Value = serde_json::from_str(text).unwrap();
        let expected: Vec<u32> = ((i + 1) as u32..=values.len() as u32).collect();
        let gte = json!({"op":"range","field":"/n","gte":bound});
        assert_eq!(ids(&mut db, gte), expected, "{text}");
        assert_eq!(ids(&mut db, eq("/n", bound)), vec![i as u32 + 1]);
    }
    let same = serde_json::from_str("1.000e0").unwrap();
    assert_eq!(ids(&mut db, eq("/n", same)), vec![12]);
    assert_eq!(ids(&mut db, eq("/n", json!(-0.0))), vec![9]);
    assert!(ids(&mut db, json!({"op":"range","field":"/n","gt":10,"lt":1})).is_empty());
    assert!(
        db.select(&p(json!({"op":"range","field":"/n","gt":1,"gte":1})))
            .is_err()
    );
}

#[test]
fn string_ranges_follow_byte_order() {
    let mut db = memory();
    let names = [
        "apple",
        "banana",
        "blueberry",
        "cherry",
        "中文",
        "2026-01-03",
        "2025-12-31",
    ];
    db.create_many(names.iter().map(|n| node(json!({"name":n}))).collect())
        .unwrap();
    db.create(node(json!({"name":5}))).unwrap();
    let range = |bounds: Value| {
        let mut query = json!({"op":"range","field":"/name"});
        query
            .as_object_mut()
            .unwrap()
            .extend(bounds.as_object().unwrap().clone());
        query
    };
    assert_eq!(ids(&mut db, range(json!({"gte":"b","lt":"c"}))), vec![2, 3]);
    assert_eq!(
        ids(&mut db, range(json!({"gt":"banana","lte":"cherry"}))),
        vec![3, 4]
    );
    assert_eq!(ids(&mut db, range(json!({"gte":"中"}))), vec![5]);
    assert_eq!(ids(&mut db, range(json!({"lt":"2026"}))), vec![7]);
    assert_eq!(
        ids(&mut db, range(json!({"gte":"2026-01-01","lt":"2027"}))),
        vec![6]
    );
    assert_eq!(ids(&mut db, range(json!({"gte":0}))), vec![8]);
    assert!(db.select(&p(range(json!({"gte":"a","lt":5})))).is_err());
    assert!(db.select(&p(range(json!({"gte":true})))).is_err());
}

#[test]
fn prefix_unicode_and_pagination() {
    let mut db = memory();
    db.create_many(nodes(vec![
        json!({"name":"小明"}),
        json!({"name":"小明同学"}),
        json!({"name":"小红"}),
        json!({"name":"a'_%"}),
        json!({"name":"\u{10ffff}x"}),
    ]))
    .unwrap();
    let prefix = |text: &str| json!({"op":"prefix","field":"/name","prefix":text});
    assert_eq!(ids(&mut db, prefix("小明")), vec![1, 2]);
    assert_eq!(ids(&mut db, prefix("a'_%")), vec![4]);
    assert_eq!(ids(&mut db, prefix("\u{10ffff}")), vec![5]);
    let mut q = Query::new(Predicate::All);
    q.limit = 2;
    q.include_data = true;
    let first = db.query(&q).unwrap();
    assert_eq!(
        (first.total, first.ids.clone(), first.next_after),
        (5, vec![1, 2], Some(2))
    );
    assert_eq!(first.nodes.unwrap()[0].attrs["name"], "小明");
    q.after = 2;
    q.include_data = false;
    assert_eq!(db.query(&q).unwrap().ids, vec![3, 4]);
    q.after = 4;
    assert_eq!(db.query(&q).unwrap().next_after, None);
    q.after = u32::MAX;
    assert!(db.query(&q).unwrap().ids.is_empty());
    q.limit = 0;
    q.after = 0;
    let result = db.query(&q).unwrap();
    assert_eq!((result.total, result.ids.len()), (5, 0));
}
