mod common;

use common::{eq, ids, memory, node};
use serde_json::{Value, json};

const DAY: i64 = 1_789_228_800_000; // 2026-09-13 00:00 +08:00
const HOUR: i64 = 3_600_000;

fn op(name: &str, field: &str, start: i64, end: i64) -> Value {
    json!({"op":name,"field":field,"start":start,"end":end})
}
fn at(field: &str, t: i64) -> Value {
    json!({"op":"at","field":field,"value":t})
}

#[test]
fn fuzzy_day_refined_to_an_hour() {
    let mut db = memory();
    let today = json!({"text":"今天","start":DAY,"end":DAY + 24 * HOUR});
    for what in ["吃饭", "逛商场", "睡觉"] {
        db.create(node(json!({"what":what,"time":today}))).unwrap();
    }
    assert_eq!(
        ids(&mut db, op("overlap", "/time", DAY, DAY + 24 * HOUR)),
        vec![1, 2, 3]
    );
    assert_eq!(
        ids(
            &mut db,
            op("overlap", "/time", DAY - 7 * 24 * HOUR, DAY + HOUR)
        ),
        vec![1, 2, 3]
    );
    assert!(
        ids(
            &mut db,
            op("overlap", "/time", DAY + 24 * HOUR, DAY + 48 * HOUR)
        )
        .is_empty()
    );
    let noon = json!({"text":"中午12点","start":DAY + 12 * HOUR,"end":DAY + 13 * HOUR});
    db.patch(1, json!({"attrs":{"time":noon}})).unwrap();
    assert_eq!(ids(&mut db, at("/time", DAY + 18 * HOUR)), vec![2, 3]);
    assert_eq!(ids(&mut db, at("/time", DAY + 12 * HOUR)), vec![1, 2, 3]);
    assert_eq!(
        ids(
            &mut db,
            op("contained_by", "/time", DAY + 11 * HOUR, DAY + 14 * HOUR)
        ),
        vec![1]
    );
    assert_eq!(
        ids(
            &mut db,
            op("contains", "/time", DAY + 20 * HOUR, DAY + 21 * HOUR)
        ),
        vec![2, 3]
    );
    assert_eq!(db.stats().unwrap().intervals, 3);
}

#[test]
fn half_open_instants_and_open_ends() {
    let mut db = memory();
    db.create_many(vec![
        node(json!({"span":{"start":10,"end":20}})),
        node(json!({"span":{"start":15,"end":15}})),
        node(json!({"span":{"start":30}})),
        node(json!({"span":{"start":40,"end":null}})),
        node(json!({"span":{"start":50,"end":45}})),
        node(json!({"span":{"start":1,"end":"later"}})),
        node(json!({"span":{"begin":1,"end":2}})),
    ])
    .unwrap();
    assert_eq!(db.stats().unwrap().intervals, 4);
    assert_eq!(ids(&mut db, at("/span", 10)), vec![1]);
    assert!(ids(&mut db, at("/span", 20)).is_empty(), "end is exclusive");
    assert_eq!(ids(&mut db, at("/span", 15)), vec![1, 2]);
    assert_eq!(ids(&mut db, at("/span", 1_000_000)), vec![3, 4]);
    assert_eq!(ids(&mut db, op("overlap", "/span", 15, 16)), vec![1, 2]);
    assert_eq!(ids(&mut db, op("overlap", "/span", 16, 30)), vec![1]);
    assert_eq!(ids(&mut db, op("overlap", "/span", 20, 31)), vec![3]);
    assert_eq!(ids(&mut db, op("contains", "/span", 12, 18)), vec![1]);
    assert_eq!(
        ids(&mut db, op("contains", "/span", 45, 1_000_000)),
        vec![3, 4]
    );
    assert_eq!(
        ids(&mut db, op("contained_by", "/span", 10, 20)),
        vec![1, 2]
    );
    assert!(
        ids(&mut db, op("contained_by", "/span", 10, 15)).is_empty(),
        "instant at the exclusive end"
    );
    assert!(
        ids(&mut db, op("contained_by", "/span", 0, 1_000_000))
            .iter()
            .all(|id| *id <= 2)
    );
    // The object fields stay ordinary attributes.
    assert_eq!(ids(&mut db, eq("/span/end", json!("later"))), vec![6]);
    assert!(db.select(&common::p(op("overlap", "/span", 5, 1))).is_err());
}

#[test]
fn intervals_in_arrays_keep_start_and_end_together() {
    let mut db = memory();
    let jobs = json!({"jobs":[{"company":"A","time":{"start":2010,"end":2012}},{"company":"B","time":{"start":2020,"end":2024}}]});
    let id = db.create(node(jobs)).unwrap();
    assert!(ids(&mut db, at("/jobs/*/time", 2015)).is_empty());
    assert_eq!(ids(&mut db, at("/jobs/*/time", 2011)), vec![id]);
    assert_eq!(ids(&mut db, at("/jobs/1/time", 2021)), vec![id]);
    assert!(ids(&mut db, at("/jobs/0/time", 2021)).is_empty());
    assert!(ids(&mut db, op("overlap", "/jobs/*/time", 2013, 2019)).is_empty());
    assert_eq!(
        ids(&mut db, op("contained_by", "/jobs/*/time", 2019, 2025)),
        vec![id]
    );
    db.patch(
        id,
        json!({"attrs":{"jobs":[{"company":"A","time":{"start":2010,"end":2016}}]}}),
    )
    .unwrap();
    assert_eq!(ids(&mut db, at("/jobs/*/time", 2015)), vec![id]);
    assert!(ids(&mut db, at("/jobs/*/time", 2021)).is_empty());
    assert_eq!(db.stats().unwrap().intervals, 2);
    db.delete(id).unwrap();
    assert_eq!(db.stats().unwrap().intervals, 0);
}

#[test]
fn candidate_probe_matches_full_scan() {
    let mut db = memory();
    let values: Vec<Value> = (0..3000)
        .map(|i| json!({"group":i % 50,"span":{"start":i,"end":i + (i % 7) * 3}}))
        .collect();
    db.create_many(values.into_iter().map(node).collect())
        .unwrap();
    for query in [
        at("/span", 1500),
        op("overlap", "/span", 100, 140),
        op("contains", "/span", 2000, 2004),
    ] {
        let full = ids(&mut db, query.clone());
        let expected: Vec<u32> = full
            .iter()
            .copied()
            .filter(|id| (id - 1) % 50 == 7)
            .collect();
        let narrowed = json!({"op":"and","args":[eq("/group",json!(7)),query]});
        assert_eq!(ids(&mut db, narrowed), expected);
        assert!(!full.is_empty());
    }
}

#[test]
fn length_bound_survives_deletes_open_ends_and_odd_numbers() {
    let mut db = memory();
    let long = db
        .create(node(json!({"t":{"start":0,"end":1000}})))
        .unwrap();
    let short: Vec<_> = (0..3000)
        .map(|i| node(json!({"t":{"start":i,"end":i + 2}})))
        .collect();
    db.create_many(short).unwrap();
    assert_eq!(ids(&mut db, at("/t", 500)), vec![long, 501, 502]);
    db.delete(long).unwrap();
    assert_eq!(ids(&mut db, at("/t", 500)), vec![501, 502]);
    let open = db.create(node(json!({"t":{"start":10}}))).unwrap();
    assert_eq!(ids(&mut db, at("/t", 2500)), vec![2501, 2502, open]);
    assert_eq!(
        ids(&mut db, op("overlap", "/t", 100, 103)),
        vec![101, 102, 103, 104, open]
    );
    assert_eq!(
        ids(&mut db, op("contains", "/t", 100, 101)),
        vec![101, 102, open]
    );
    assert_eq!(
        ids(&mut db, op("contained_by", "/t", 100, 110)),
        (102..=110).collect::<Vec<_>>()
    );

    let mut db = memory();
    let odd: Vec<Value> = [
        r#"{"t":{"start":-1.5,"end":-0.25}}"#,
        r#"{"t":{"start":1e-30,"end":2e-30}}"#,
        r#"{"t":{"start":1e300,"end":1e301}}"#,
    ]
    .iter()
    .map(|s| serde_json::from_str(s).unwrap())
    .collect();
    db.create_many(odd.into_iter().map(node).collect()).unwrap();
    let query = |text: &str| serde_json::from_str::<Value>(text).unwrap();
    assert_eq!(ids(&mut db, at("/t", -1)), vec![1]);
    assert_eq!(
        ids(
            &mut db,
            query(r#"{"op":"at","field":"/t","value":1.5e-30}"#)
        ),
        vec![2]
    );
    assert_eq!(
        ids(&mut db, query(r#"{"op":"at","field":"/t","value":5e300}"#)),
        vec![3]
    );
    // Beyond f64 the path loses its length bound and falls back to a full scan.
    db.create(node(query(r#"{"t":{"start":1e400,"end":3e400}}"#)))
        .unwrap();
    assert_eq!(
        ids(&mut db, query(r#"{"op":"at","field":"/t","value":2e400}"#)),
        vec![4]
    );
    assert_eq!(ids(&mut db, at("/t", -1)), vec![1]);
}
