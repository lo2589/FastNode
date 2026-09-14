mod common;

use common::{Temp, ids, memory, p};
use fastnode::{NewNode, Store};
use serde_json::{Value, json};

fn define(db: &mut Store) {
    db.define_index(
        "timeline",
        "state",
        &["/timeline", "/recorded/end"],
        "/valid/start",
    )
    .unwrap();
}

fn version(db: &mut Store, timeline: &str, start: Value, value: &str) -> u32 {
    let attrs = json!({"timeline":timeline,"value":value,"valid":{"start":start,"end":null},"recorded":{"start":0,"end":null}});
    db.create(NewNode::new("state", value, attrs)).unwrap()
}

fn seek(group: Value, options: Value) -> Value {
    let mut seek = json!({"op":"seek","index":"timeline","group":group});
    seek.as_object_mut()
        .unwrap()
        .extend(options.as_object().unwrap().clone());
    seek
}

fn live(key: &str) -> Value {
    json!([key, null])
}

#[test]
fn seek_walks_one_group_in_order() {
    let mut db = memory();
    define(&mut db);
    let a1 = version(&mut db, "ball#condition", json!(100), "完好");
    let a2 = version(&mut db, "ball#condition", json!(200), "坏了");
    let a3 = version(&mut db, "ball#condition", json!(300), "修好");
    let b1 = version(&mut db, "ming#mood", json!(150), "开心");
    let latest = |t: i64| {
        seek(
            live("ball#condition"),
            json!({"lte":t,"direction":"desc","limit":1}),
        )
    };
    assert_eq!(ids(&mut db, latest(250)), vec![a2]);
    assert_eq!(ids(&mut db, latest(200)), vec![a2]);
    assert!(ids(&mut db, latest(99)).is_empty());
    let before = seek(
        live("ball#condition"),
        json!({"lt":200,"direction":"desc","limit":1}),
    );
    assert_eq!(ids(&mut db, before), vec![a1]);
    let next = seek(live("ball#condition"), json!({"gt":200,"limit":1}));
    assert_eq!(ids(&mut db, next), vec![a3]);
    assert_eq!(
        ids(&mut db, seek(live("ball#condition"), json!({}))),
        vec![a1, a2, a3]
    );
    assert_eq!(ids(&mut db, seek(live("ming#mood"), json!({}))), vec![b1]);

    // Retiring a version moves it to another group; deleting drops it.
    db.patch(a2, json!({"attrs":{"recorded":{"end":999}}}))
        .unwrap();
    assert_eq!(ids(&mut db, latest(250)), vec![a1]);
    assert_eq!(
        ids(&mut db, seek(json!(["ball#condition", 999]), json!({}))),
        vec![a2]
    );
    db.delete(a1).unwrap();
    assert!(ids(&mut db, latest(250)).is_empty());

    let with_value = json!({"op":"and","args":[seek(live("ball#condition"), json!({"lte":400,"direction":"desc","limit":2})),{"op":"eq","field":"/value","value":"修好"}]});
    assert_eq!(ids(&mut db, with_value), vec![a3]);
}

#[test]
fn existing_nodes_are_backfilled_and_only_single_values_are_indexed() {
    let mut db = memory();
    let early = version(&mut db, "t", json!(10), "number");
    let text = version(&mut db, "t", json!("later"), "string");
    let open = json!({"start":20});
    db.create(NewNode::new(
        "state",
        "no timeline",
        json!({"valid":open,"recorded":{"end":null}}),
    ))
    .unwrap();
    db.create(NewNode::new(
        "state",
        "two timelines",
        json!({"timeline":["t","u"],"valid":{"start":30},"recorded":{"end":null}}),
    ))
    .unwrap();
    db.create(NewNode::new(
        "other",
        "other type",
        json!({"timeline":"t","valid":{"start":40},"recorded":{"end":null}}),
    ))
    .unwrap();
    define(&mut db);
    assert_eq!(ids(&mut db, seek(live("t"), json!({}))), vec![early, text]);
    let newest_number = seek(live("t"), json!({"gte":0,"direction":"desc","limit":1}));
    assert_eq!(
        ids(&mut db, newest_number),
        vec![early],
        "a number bound never reaches strings"
    );
    assert_eq!(
        ids(&mut db, seek(live("t"), json!({"gte":"a"}))),
        vec![text]
    );

    define(&mut db);
    assert!(
        db.define_index("timeline", "state", &["/timeline"], "/valid/start")
            .is_err()
    );
    assert!(db.define_index("", "state", &["/x"], "/y").is_err());
    assert!(
        db.select(&p(json!({"op":"seek","index":"missing","group":["t"]})))
            .is_err()
    );
    assert!(
        db.select(&p(seek(json!(["t"]), json!({})))).is_err(),
        "group arity"
    );
    assert!(
        db.select(&p(seek(live("t"), json!({"gte":1,"lt":"z"}))))
            .is_err()
    );
}

#[test]
fn seek_inside_a_write_sees_uncommitted_versions_and_rollback_removes_them() {
    let mut db = memory();
    define(&mut db);
    let failed = db.write(|w| -> anyhow::Result<()> {
        let attrs = json!({"timeline":"t","valid":{"start":5},"recorded":{"end":null}});
        let id = w.create(NewNode::new("state", "v", attrs))?;
        let seen = w.select(&p(seek(
            live("t"),
            json!({"lte":5,"direction":"desc","limit":1}),
        )))?;
        assert_eq!(seen.iter().collect::<Vec<_>>(), vec![id]);
        anyhow::bail!("roll back")
    });
    assert!(failed.is_err());
    assert!(ids(&mut db, seek(live("t"), json!({}))).is_empty());
}

#[test]
fn definitions_and_entries_survive_reopening() {
    let path = Temp::new();
    let id = {
        let mut db = Store::open(&path.0).unwrap();
        define(&mut db);
        version(&mut db, "t", json!(7), "v")
    };
    let mut db = Store::open(&path.0).unwrap();
    assert_eq!(ids(&mut db, seek(live("t"), json!({"lte":7}))), vec![id]);
    let later = version(&mut db, "t", json!(8), "w");
    assert_eq!(
        ids(
            &mut db,
            seek(live("t"), json!({"direction":"desc","limit":1}))
        ),
        vec![later]
    );
}
