mod common;

use common::{Temp, eq, ids, node, p};
use fastnode::{Query, Store};
use serde_json::json;

#[test]
fn persistence_and_multi_connection_visibility() {
    let path = Temp::new();
    let mut first = Store::open(&path.0).unwrap();
    let id = first.create(node(json!({"state":"old"}))).unwrap();
    let mut second = Store::open(&path.0).unwrap();
    assert_eq!(ids(&mut second, eq("/state", json!("old"))), vec![id]);
    first.patch(id, json!({"attrs":{"state":"new"}})).unwrap();
    assert!(ids(&mut second, eq("/state", json!("old"))).is_empty());
    assert_eq!(ids(&mut second, eq("/state", json!("new"))), vec![id]);
    drop(first);
    drop(second);
    let mut reopened = Store::open(&path.0).unwrap();
    assert_eq!(ids(&mut reopened, eq("/state", json!("new"))), vec![id]);
    reopened.delete(id).unwrap();
    assert_eq!(reopened.stats().unwrap().nodes, 0);
}

#[test]
fn older_schemas_are_rejected() {
    for version in [1, 2, 3] {
        let path = Temp::new();
        rusqlite::Connection::open(&path.0)
            .unwrap()
            .execute_batch(&format!("PRAGMA user_version={version}"))
            .unwrap();
        let error = Store::open(&path.0).err().unwrap();
        assert!(format!("{error:#}").contains(&format!("schema v{version}")));
    }
}

#[test]
fn query_payload_uses_same_snapshot_as_bitmap_under_concurrent_writes() {
    let path = Temp::new();
    let mut db = Store::open(&path.0).unwrap();
    db.create(node(json!({"state":"a"}))).unwrap();
    let write_path = path.0.clone();
    let writer = std::thread::spawn(move || {
        let mut db = Store::open(write_path).unwrap();
        for i in 0..100 {
            let state = if i % 2 == 0 { "b" } else { "a" };
            db.patch(1, json!({"attrs":{"state":state}})).unwrap();
        }
    });
    let mut query = Query::new(p(eq("/state", json!("a"))));
    query.limit = 10;
    query.include_data = true;
    for _ in 0..100 {
        let result = db.query(&query).unwrap();
        for node in result.nodes.unwrap() {
            assert_eq!(node.attrs["state"], "a");
        }
    }
    writer.join().unwrap();
}
