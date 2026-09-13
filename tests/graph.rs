mod common;

use common::{eq, ids, link, memory, node, p};
use fastnode::{Direction, Link, LinkMode, LinkOptions, NewNode, Query};
use serde_json::{Value, json};

#[test]
fn multi_hop_direction_filters_cycles_and_deletion() {
    let mut db = memory();
    db.create_many(vec![
        NewNode::new("person", "小明", json!({"name":"小明"})),
        NewNode::new("event", "2026 活动", json!({"year":2026})),
        NewNode::new("event", "2025 活动", json!({"year":2025})),
        NewNode::new("place", "上海会场", json!({"city":"上海"})),
        NewNode::new("place", "北京会场", json!({"city":"北京"})),
    ])
    .unwrap();
    for (from, relation, to) in [
        (1, "event", 2),
        (1, "event", 3),
        (2, "place", 4),
        (3, "place", 5),
        (4, "back", 1),
    ] {
        let edge = Link {
            from,
            relation: relation.into(),
            to,
        };
        assert!(db.link(&edge).unwrap());
        assert!(!db.link(&edge).unwrap());
    }
    let traverse = json!({"op":"traverse","from":eq("/name",json!("小明")),"steps":[{"relation":"event","filter":eq("/year",json!(2026))},{"relation":"place"}]});
    assert_eq!(ids(&mut db, traverse.clone()), vec![4]);
    assert_eq!(
        ids(
            &mut db,
            json!({"op":"and","args":[eq("/city",json!("上海")),traverse]})
        ),
        vec![4]
    );
    let back = json!({"op":"traverse","from":{"op":"ids","ids":[4]},"steps":[{"relation":"place","direction":"in"},{"relation":"event","direction":"in"}]});
    assert_eq!(ids(&mut db, back), vec![1]);
    assert_eq!(
        ids(
            &mut db,
            json!({"op":"has_link","relation":"event","target":2})
        ),
        vec![1]
    );
    let pointed = json!({"op":"has_link","relation":"event","target":1,"direction":"in"});
    assert_eq!(ids(&mut db, pointed), vec![2, 3]);
    let shanghai = json!({"op":"traverse","from":{"op":"ids","ids":[1,2,3]},"steps":[{"relation":"place","filter":eq("/city",json!("上海"))}]});
    assert_eq!(ids(&mut db, shanghai), vec![4]);
    let cycle = json!({"op":"traverse","from":{"op":"ids","ids":[1]},"steps":[{"relation":"event"},{"relation":"place"},{"relation":"back"}]});
    assert_eq!(ids(&mut db, cycle), vec![1]);
    db.delete(2).unwrap();
    assert!(
        ids(
            &mut db,
            json!({"op":"has_link","relation":"event","target":2})
        )
        .is_empty()
    );
    assert!(db.links(2).unwrap().is_empty());
    assert_eq!(db.stats().unwrap().links, 3);
}

#[test]
fn repeated_hops_follow_one_relation_many_levels() {
    let mut db = memory();
    db.create_many(vec![
        node(json!({"n":1})),
        node(json!({"n":2})),
        NewNode::new("stop", "a stop", json!({"n":3})),
        node(json!({"n":4})),
        node(json!({"n":5})),
    ])
    .unwrap();
    for from in 1..5 {
        link(&mut db, from, "next", from + 1);
    }
    let walk = |step: Value| json!({"op":"traverse","from":{"op":"ids","ids":[1]},"steps":[step]});
    assert_eq!(
        ids(&mut db, walk(json!({"relation":"next","min":1,"max":2}))),
        vec![2, 3]
    );
    assert_eq!(
        ids(&mut db, walk(json!({"relation":"next","min":0,"max":1}))),
        vec![1, 2]
    );
    assert_eq!(
        ids(&mut db, walk(json!({"relation":"next","min":3}))),
        vec![4, 5]
    );
    assert_eq!(
        ids(&mut db, walk(json!({"relation":"next","max":10}))),
        vec![2, 3, 4, 5]
    );
    // The filter selects reached Nodes; hops still pass through 2.
    let stop = walk(json!({"relation":"next","min":1,"filter":eq("@type",json!("stop"))}));
    assert_eq!(ids(&mut db, stop), vec![3]);
    // A cycle terminates; the start is at distance zero.
    link(&mut db, 5, "next", 1);
    assert_eq!(
        ids(&mut db, walk(json!({"relation":"next","min":1}))),
        vec![2, 3, 4, 5]
    );
    let backwards = json!({"op":"traverse","from":{"op":"ids","ids":[4]},"steps":[{"relation":"next","direction":"in","min":1,"max":2}]});
    assert_eq!(ids(&mut db, backwards), vec![2, 3]);
    let narrowed = json!({"op":"and","args":[eq("/n",json!(4)),{"op":"traverse","from":{"op":"ids","ids":[1]},"steps":[{"relation":"next"},{"relation":"next","min":1}]}]});
    assert_eq!(ids(&mut db, narrowed), vec![4]);
    assert!(
        db.select(&p(walk(json!({"relation":"next","min":3,"max":2}))))
            .is_err()
    );
}

#[test]
fn get_pulls_links_at_three_levels() {
    let mut db = memory();
    db.create_many(vec![
        NewNode::new("tree", "root", json!({"name":"root"})),
        NewNode::new("tree", "child a", json!({"name":"a"})),
        NewNode::new("tree", "child b", json!({"name":"b"})),
        NewNode::new("doc", "cites root", json!({"title":"doc"})),
    ])
    .unwrap();
    link(&mut db, 1, "contains", 2);
    link(&mut db, 1, "contains", 3);
    link(&mut db, 4, "cites", 1);
    let options = |mode, limit| LinkOptions { mode, limit };
    let links_of = |db: &mut fastnode::Store, o: LinkOptions| {
        db.get_with(1, &o).unwrap().unwrap().links.unwrap()
    };

    let none = links_of(&mut db, options(LinkMode::None, 100));
    assert_eq!(
        none.out.iter().map(|l| l.id).collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert!(
        none.out
            .iter()
            .all(|l| l.summary.is_none() && l.kind.is_none())
    );
    assert_eq!(
        (none.incoming[0].relation.as_str(), none.incoming[0].id),
        ("cites", 4)
    );

    let summary = links_of(&mut db, LinkOptions::default());
    assert_eq!(summary.out[0].summary.as_deref(), Some("child a"));
    assert_eq!(summary.out[0].kind.as_deref(), Some("tree"));
    assert!(summary.out[0].attrs.is_none());
    assert_eq!(summary.incoming[0].summary.as_deref(), Some("cites root"));

    let full = links_of(&mut db, options(LinkMode::Full, 100));
    assert_eq!(full.out[1].attrs, Some(json!({"name":"b"})));

    let limited = links_of(&mut db, options(LinkMode::Summary, 1));
    assert_eq!((limited.out.len(), limited.out_more), (1, true));
    assert_eq!((limited.incoming.len(), limited.in_more), (1, false));

    // A summary change is visible through every reference at once.
    db.patch(2, json!({"summary":"renamed a"})).unwrap();
    let mut query = Query::new(p(eq("/title", json!("doc"))));
    query.include_data = true;
    let doc_links = db.query(&query).unwrap().nodes.unwrap()[0]
        .links
        .clone()
        .unwrap();
    assert_eq!(doc_links.out[0].summary.as_deref(), Some("root"));
    let root = links_of(&mut db, LinkOptions::default());
    assert_eq!(root.out[0].summary.as_deref(), Some("renamed a"));
    assert!(db.get(1).unwrap().unwrap().links.is_none());
    assert!(db.get_with(99, &LinkOptions::default()).unwrap().is_none());
    assert!(db.get_with(1, &options(LinkMode::None, 1_000_000)).is_err());
}

#[test]
fn neighbors_reads_one_relation() {
    let mut db = memory();
    db.create_many((0..5).map(|i| node(json!({"i":i}))).collect())
        .unwrap();
    link(&mut db, 1, "a", 3);
    link(&mut db, 1, "b", 2);
    link(&mut db, 1, "a", 2);
    link(&mut db, 4, "a", 1);
    link(&mut db, 5, "b", 1);
    let options = LinkOptions {
        mode: LinkMode::None,
        limit: 1,
    };
    let (out, more) = db.neighbors(1, "a", Direction::Out, &options).unwrap();
    assert_eq!(
        (out.iter().map(|l| l.id).collect::<Vec<_>>(), more),
        (vec![2], true)
    );
    let (incoming, more) = db
        .neighbors(1, "b", Direction::In, &LinkOptions::default())
        .unwrap();
    assert_eq!(
        (incoming[0].id, incoming[0].summary.as_deref(), more),
        (5, Some("a test node"), false)
    );
    assert!(
        db.neighbors(1, "missing", Direction::Out, &LinkOptions::default())
            .unwrap()
            .0
            .is_empty()
    );
}
