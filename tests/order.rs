mod common;

use common::{eq, memory, node, nodes, ordered, p};
use fastnode::{OrderBy, Predicate, Query};
use serde_json::json;

fn all() -> serde_json::Value {
    json!({"op":"all"})
}

#[test]
fn numbers_then_strings_then_missing_in_both_directions() {
    let mut db = memory();
    db.create_many(nodes(vec![
        json!({"v":3}),
        json!({"v":"b"}),
        json!({}),
        json!({"v":1}),
        json!({"v":"a"}),
        json!({"v":3}),
        json!({"v":true}),
        json!({"v":null}),
    ]))
    .unwrap();
    let asc = vec![4, 1, 6, 5, 2, 3, 7, 8];
    let desc = vec![2, 5, 6, 1, 4, 3, 7, 8];
    for page in [1, 3, 100] {
        assert_eq!(
            ordered(&mut db, all(), OrderBy::asc("/v"), page),
            asc,
            "page {page}"
        );
        assert_eq!(
            ordered(&mut db, all(), OrderBy::desc("/v"), page),
            desc,
            "page {page}"
        );
    }
    let only_numbers = json!({"op":"range","field":"/v"});
    assert_eq!(
        ordered(&mut db, only_numbers, OrderBy::desc("/v"), 2),
        vec![6, 1, 4]
    );
}

#[test]
fn ordered_page_carries_total_and_nodes_in_order() {
    let mut db = memory();
    db.create_many(
        (0..10)
            .map(|i| node(json!({"rank":(i * 7) % 10,"even":i % 2 == 0})))
            .collect(),
    )
    .unwrap();
    let mut query = Query::new(p(eq("/even", json!(true))));
    query.order_by = Some(OrderBy::desc("/rank"));
    query.limit = 2;
    query.include_data = true;
    let first = db.query(&query).unwrap();
    assert_eq!(first.total, 5);
    assert_eq!(first.ids, vec![5, 9]);
    let ranks: Vec<_> = first
        .nodes
        .unwrap()
        .iter()
        .map(|n| n.attrs["rank"].clone())
        .collect();
    assert_eq!(ranks, vec![json!(8), json!(6)]);
    query.cursor = first.next_cursor;
    assert_eq!(db.query(&query).unwrap().ids, vec![3, 7]);
    query.limit = 0;
    query.cursor = None;
    let count = db.query(&query).unwrap();
    assert_eq!(
        (count.total, count.ids.len(), count.next_cursor),
        (5, 0, None)
    );
}

#[test]
fn multi_valued_paths_cannot_be_sorted() {
    let mut db = memory();
    let tags = db
        .create(node(json!({"tags":[1,2],"one":[5],"jobs":[{"at":1}]})))
        .unwrap();
    db.create(node(json!({"one":[3],"jobs":[{"at":2},{"at":3}]})))
        .unwrap();
    let sort = |db: &mut fastnode::Store, field: &str| {
        let mut query = Query::new(Predicate::All);
        query.order_by = Some(OrderBy::asc(field));
        db.query(&query).map(|r| r.ids)
    };
    let error = sort(&mut db, "/tags").unwrap_err();
    assert!(format!("{error:#}").contains("single-valued"));
    assert!(sort(&mut db, "/jobs/*/at").is_err());
    assert_eq!(sort(&mut db, "/one").unwrap(), vec![2, 1]);
    assert_eq!(sort(&mut db, "/jobs/0/at").unwrap(), vec![1, 2]);
    db.patch(tags, json!({"attrs":{"tags":[9]}})).unwrap();
    assert_eq!(sort(&mut db, "/tags").unwrap(), vec![1, 2]);
    db.patch(2, json!({"attrs":{"jobs":[{"at":2}]}})).unwrap();
    assert_eq!(sort(&mut db, "/jobs/*/at").unwrap(), vec![1, 2]);
}

#[test]
fn ordering_is_validated() {
    let mut db = memory();
    db.create(node(json!({"v":1}))).unwrap();
    let mut query = Query::new(Predicate::All);
    query.cursor = Some("0.00.1".into());
    assert!(db.query(&query).is_err(), "cursor without order_by");
    query.order_by = Some(OrderBy::asc("/v"));
    query.cursor = None;
    query.after = 1;
    assert!(db.query(&query).is_err(), "after with order_by");
    query.after = 0;
    for bad in ["x", "9.00.1", "1.zz.1", "1.ff.1", "0.00.1.2"] {
        query.cursor = Some(bad.into());
        assert!(db.query(&query).is_err(), "{bad}");
    }
    query.order_by = Some(OrderBy::asc("city"));
    query.cursor = None;
    assert!(db.query(&query).is_err(), "field must be a pointer");
}

#[test]
fn type_and_string_order_use_utf8_bytes() {
    let mut db = memory();
    for (kind, name) in [
        ("person", "张三"),
        ("event", "Beta"),
        ("person", "alpha"),
        ("place", "Alpha"),
    ] {
        db.create(fastnode::NewNode::new(kind, name, json!({"name":name})))
            .unwrap();
    }
    assert_eq!(
        ordered(&mut db, all(), OrderBy::asc("/name"), 10),
        vec![4, 2, 3, 1]
    );
    assert_eq!(
        ordered(&mut db, all(), OrderBy::asc("@type"), 1),
        vec![2, 1, 3, 4]
    );
}
