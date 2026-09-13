mod common;

use common::{eq, ids, memory, node};
use fastnode::{Link, NewNode};
use serde_json::json;

#[test]
fn crud_updates_every_index_and_preserves_json() {
    let mut db = memory();
    let attrs = json!({"city":"上海","age":26,"skills":["rust","python","rust"],"profile":{"a/b":{"~name":"小明"}},"nullable":null});
    let id = db
        .create(NewNode::new("person", "小明", attrs.clone()))
        .unwrap();
    let stored = db.get(id).unwrap().unwrap();
    assert_eq!(
        (stored.kind.as_str(), stored.summary.as_str(), &stored.attrs),
        ("person", "小明", &attrs)
    );
    assert_eq!(ids(&mut db, eq("@type", json!("person"))), vec![id]);
    assert_eq!(ids(&mut db, eq("/skills", json!("rust"))), vec![id]);
    assert_eq!(
        ids(&mut db, eq("/profile/a~1b/~0name", json!("小明"))),
        vec![id]
    );
    assert_eq!(ids(&mut db, eq("/skills/1", json!("python"))), vec![id]);
    db.patch(
        id,
        json!({"type":"engineer","attrs":{"city":"北京","skills":["cuda"],"age":30,"nullable":null}}),
    )
    .unwrap();
    assert!(ids(&mut db, eq("@type", json!("person"))).is_empty());
    assert_eq!(ids(&mut db, eq("@type", json!("engineer"))), vec![id]);
    assert!(ids(&mut db, eq("/city", json!("上海"))).is_empty());
    assert!(ids(&mut db, eq("/skills", json!("rust"))).is_empty());
    assert!(ids(&mut db, json!({"op":"exists","field":"/nullable"})).is_empty());
    let twenties = json!({"op":"range","field":"/age","gte":20,"lt":30});
    assert!(ids(&mut db, twenties).is_empty());
    let thirty = json!({"op":"range","field":"/age","gte":30,"lte":30});
    assert_eq!(ids(&mut db, thirty), vec![id]);
    let patched = db.get(id).unwrap().unwrap();
    assert_eq!((patched.version, patched.summary.as_str()), (2, "小明"));
    db.replace(id, node(json!({"new":true}))).unwrap();
    assert!(ids(&mut db, eq("/skills", json!("cuda"))).is_empty());
    assert!(db.delete(id).unwrap());
    assert!(!db.delete(id).unwrap());
    assert!(ids(&mut db, json!({"op":"all"})).is_empty());
    assert_eq!(db.stats().unwrap().bitmap_blocks, 0);
    assert!(db.create(node(json!({}))).unwrap() > id);
}

#[test]
fn node_requires_type_summary_and_object_attrs() {
    let mut db = memory();
    assert!(db.create(NewNode::new("person", "", json!({}))).is_err());
    assert!(db.create(NewNode::new("person", " \n", json!({}))).is_err());
    assert!(db.create(NewNode::new("", "summary", json!({}))).is_err());
    assert!(
        db.create(NewNode::new("person", "summary", json!([1])))
            .is_err()
    );
    let id = db.create(node(json!({"x":1}))).unwrap();
    assert!(db.patch(id, json!({"summary":null})).is_err());
    assert!(db.patch(id, json!({"x":2})).is_err());
    assert!(db.patch(id, json!({"summary":""})).is_err());
    db.patch(id, json!({"summary":"renamed"})).unwrap();
    assert_eq!(db.get(id).unwrap().unwrap().summary, "renamed");
    assert_eq!(ids(&mut db, eq("/x", json!(1))), vec![id]);
    let input = r#"{"type":"t","summary":"ok"} {"type":"t","attrs":{}}"#;
    let error = db.import(input.as_bytes(), 10).unwrap_err();
    assert!(format!("{error:#}").contains("record 2"));
    assert_eq!(db.stats().unwrap().nodes, 1);
}

#[test]
fn atomic_batch_rolls_back_payload_indexes_and_edges() {
    let mut db = memory();
    let invalid = NewNode::new("t", "s", json!("invalid"));
    assert!(db.create_many(vec![node(json!({"x":1})), invalid]).is_err());
    assert_eq!(db.stats().unwrap().nodes, 0);
    let error = db.write(|w| {
        let id = w.create(node(
            json!({"city":"上海","age":23,"span":{"start":1,"end":2}}),
        ))?;
        w.link(&Link {
            from: id,
            relation: "knows".into(),
            to: 999,
        })
    });
    assert!(error.is_err());
    assert!(ids(&mut db, json!({"op":"all"})).is_empty());
    assert!(ids(&mut db, json!({"op":"range","field":"/age"})).is_empty());
    assert_eq!(db.stats().unwrap().intervals, 0);
    let id = db.create(node(json!({"x":1}))).unwrap();
    assert_eq!(id, 1);
    let aborted = db.write(|w| -> anyhow::Result<()> {
        w.patch(id, json!({"attrs":{"x":2}}))?;
        anyhow::bail!("abort")
    });
    assert!(aborted.is_err());
    assert_eq!(ids(&mut db, eq("/x", json!(1))), vec![id]);
}

#[test]
fn multiple_changes_in_one_transaction_have_correct_counts() {
    let mut db = memory();
    db.write(|w| {
        let a = w.create(node(json!({"x":1})))?;
        let b = w.create(node(json!({"x":1})))?;
        w.patch(a, json!({"attrs":{"x":2}}))?;
        w.delete(b)?;
        w.patch(a, json!({"attrs":{"x":1}}))?;
        Ok(())
    })
    .unwrap();
    assert_eq!(ids(&mut db, eq("/x", json!(1))), vec![1]);
    assert!(ids(&mut db, eq("/x", json!(2))).is_empty());
    assert_eq!(db.stats().unwrap().nodes, 1);
}

#[test]
fn select_inside_a_write_sees_uncommitted_changes() {
    let mut db = memory();
    db.write(|w| {
        let a = w.create(node(json!({"x":1})))?;
        assert_eq!(w.select(&common::p(eq("/x", json!(1))))?.len(), 1);
        w.patch(a, json!({"attrs":{"x":2}}))?;
        assert!(w.select(&common::p(eq("/x", json!(1))))?.is_empty());
        w.create(node(json!({"x":2})))?;
        assert_eq!(w.select(&common::p(eq("/x", json!(2))))?.len(), 2);
        Ok(())
    })
    .unwrap();
    assert_eq!(ids(&mut db, eq("/x", json!(2))), vec![1, 2]);
    assert!(ids(&mut db, eq("/x", json!(1))).is_empty());
}

#[test]
fn stream_import_formats_and_partial_failure_report() {
    let record = |a: u32| format!(r#"{{"type":"t","summary":"s{a}","attrs":{{"a":{a}}}}}"#);
    let mut db = memory();
    let array = format!("  [{},{},{}]", record(1), record(2), record(3));
    assert_eq!(db.import(array.as_bytes(), 2).unwrap().imported, 3);
    let lines = format!("{}\n{}\n", record(4), record(5));
    assert_eq!(db.import(lines.as_bytes(), 2).unwrap().imported, 2);
    let broken = format!("[{},{},{},false]", record(6), record(7), record(8));
    let error = db.import(broken.as_bytes(), 2).unwrap_err();
    assert!(format!("{error:#}").contains("2 nodes in 1 batches already committed"));
    assert_eq!(db.stats().unwrap().nodes, 7);
    assert!(ids(&mut db, eq("/a", json!(8))).is_empty());
    assert!(db.import("[] junk".as_bytes(), 2).is_err());
    assert!(db.import("[]".as_bytes(), 0).is_err());
}
