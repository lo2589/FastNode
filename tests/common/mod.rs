#![allow(dead_code)]

use fastnode::{Link, NewNode, OrderBy, Predicate, Query, Store};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

pub fn p(v: Value) -> Predicate {
    serde_json::from_value(v).unwrap()
}

pub fn ids(store: &mut Store, value: Value) -> Vec<u32> {
    store.select(&p(value)).unwrap().iter().collect()
}

pub fn eq(field: &str, value: Value) -> Value {
    json!({"op":"eq","field":field,"value":value})
}

pub fn node(attrs: Value) -> NewNode {
    NewNode::new("thing", "a test node", attrs)
}

pub fn nodes(attrs: Vec<Value>) -> Vec<NewNode> {
    attrs.into_iter().map(node).collect()
}

pub fn link(db: &mut Store, from: u32, relation: &str, to: u32) {
    db.link(&Link {
        from,
        relation: relation.into(),
        to,
    })
    .unwrap();
}

pub fn memory() -> Store {
    Store::open(":memory:").unwrap()
}

/// Every id of an ordered query, following next_cursor page by page.
pub fn ordered(db: &mut Store, predicate: Value, order: OrderBy, page: usize) -> Vec<u32> {
    let mut query = Query::new(p(predicate));
    query.order_by = Some(order);
    query.limit = page;
    let mut out = Vec::new();
    loop {
        let result = db.query(&query).unwrap();
        assert_eq!(result.next_after, None);
        assert!(result.ids.len() <= page);
        out.extend(result.ids);
        match result.next_cursor {
            Some(cursor) => query.cursor = Some(cursor),
            None => break,
        }
    }
    out
}

pub struct Temp(pub PathBuf);

impl Temp {
    pub fn new() -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        Self(std::env::temp_dir().join(format!(
            "fastnode-test-{}-{}.db",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}
