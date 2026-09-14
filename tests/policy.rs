mod common;

use common::{Temp, memory};
use fastnode::{NewNode, Store, WritePolicy};
use serde_json::json;

fn append_only(db: &mut Store) {
    db.set_policy("state", &WritePolicy::append_only(&["/recorded/end"]))
        .unwrap();
}

fn state(db: &mut Store) -> u32 {
    let attrs = json!({"value":"完好","recorded":{"start":1,"end":null}});
    db.create(NewNode::new("state", "完好", attrs)).unwrap()
}

fn error_of(result: anyhow::Result<impl std::fmt::Debug>) -> String {
    format!("{:#}", result.unwrap_err())
}

#[test]
fn append_only_types_cannot_be_deleted_replaced_or_rewritten() {
    let mut db = memory();
    append_only(&mut db);
    let id = state(&mut db);
    assert!(error_of(db.delete(id)).contains("cannot be deleted"));
    assert!(
        error_of(db.replace(id, NewNode::new("state", "x", json!({}))))
            .contains("cannot be replaced")
    );
    assert!(error_of(db.patch(id, json!({"attrs":{"value":"坏了"}}))).contains("/value"));
    assert!(db.patch(id, json!({"summary":"改名"})).is_err());
    assert!(db.patch(id, json!({"type":"other"})).is_err());
    assert!(
        db.patch(id, json!({"attrs":{"recorded":{"start":5}}}))
            .is_err()
    );
    assert!(db.patch(id, json!({"attrs":null})).is_err());
    db.patch(id, json!({"attrs":{"recorded":{"end":9}}}))
        .unwrap();
    db.patch(id, json!({"attrs":{}})).unwrap();
    let node = db.get(id).unwrap().unwrap();
    assert_eq!(
        node.attrs,
        json!({"value":"完好","recorded":{"start":1,"end":9}})
    );

    let other = db
        .create(NewNode::new("note", "n", json!({"x":1})))
        .unwrap();
    db.patch(other, json!({"attrs":{"x":2}})).unwrap();
    assert!(
        db.delete(other).unwrap(),
        "types without a policy stay unrestricted"
    );
}

#[test]
fn a_violation_inside_a_write_rolls_the_whole_write_back() {
    let mut db = memory();
    append_only(&mut db);
    let id = state(&mut db);
    let result = db.write(|w| {
        w.patch(id, json!({"attrs":{"recorded":{"end":3}}}))?;
        w.delete(id)?;
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(
        db.get(id).unwrap().unwrap().attrs["recorded"]["end"],
        json!(null)
    );
}

#[test]
fn policies_persist_and_can_be_relaxed() {
    let path = Temp::new();
    {
        let mut db = Store::open(&path.0).unwrap();
        append_only(&mut db);
        state(&mut db);
    }
    let mut db = Store::open(&path.0).unwrap();
    assert_eq!(
        db.policy("state").unwrap(),
        WritePolicy::append_only(&["/recorded/end"])
    );
    assert_eq!(db.policy("note").unwrap(), WritePolicy::default());
    assert!(db.delete(1).is_err());
    db.set_policy("state", &WritePolicy::default()).unwrap();
    assert!(db.delete(1).unwrap());
    assert!(
        db.set_policy("state", &WritePolicy::append_only(&["recorded"]))
            .is_err()
    );
    assert!(db.set_policy("", &WritePolicy::default()).is_err());
}
