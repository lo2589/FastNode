mod common;

use common::{Temp, eq, memory, p};
use fastnode::{Direction, Link, LinkOptions, NewNode, OrderBy, Query, Store};
use serde_json::json;

#[test]
fn ordered_queries_and_links_inside_a_write_see_uncommitted_nodes() {
    let mut db = memory();
    db.write(|w| {
        let late = w.create(NewNode::new("state", "late", json!({"k":"t","start":30})))?;
        let early = w.create(NewNode::new("state", "early", json!({"k":"t","start":10})))?;
        let middle = w.create(NewNode::new("state", "middle", json!({"k":"t","start":20})))?;
        w.link(&Link {
            from: late,
            relation: "next".into(),
            to: early,
        })?;

        let mut query = Query::new(p(eq("/k", json!("t"))));
        query.order_by = Some(OrderBy::asc("/start"));
        query.include_data = true;
        let result = w.query(&query)?;
        assert_eq!(result.ids, vec![early, middle, late]);
        assert_eq!(result.nodes.unwrap()[0].summary, "early");

        let node = w.get_with(late, &LinkOptions::default())?.unwrap();
        assert_eq!(node.links.unwrap().out[0].summary.as_deref(), Some("early"));
        let (refs, more) = w.neighbors(early, "next", Direction::In, &LinkOptions::default())?;
        assert_eq!((refs[0].id, more), (late, false));
        Ok(())
    })
    .unwrap();
}

#[test]
fn transaction_time_is_fixed_per_write_and_strictly_increasing() {
    let path = Temp::new();
    let mut db = Store::open(&path.0).unwrap();
    let (first, again) = db.write(|w| Ok((w.now()?, w.now()?))).unwrap();
    assert_eq!(first, again);
    let mut last = first;
    for _ in 0..50 {
        let now = db.write(|w| w.now()).unwrap();
        assert!(now > last);
        last = now;
    }
    drop(db);
    let mut reopened = Store::open(&path.0).unwrap();
    let rolled_back = reopened.write(|w| -> anyhow::Result<i64> {
        let now = w.now()?;
        anyhow::bail!("discard {now}")
    });
    assert!(rolled_back.is_err());
    assert!(reopened.write(|w| w.now()).unwrap() > last);
}
