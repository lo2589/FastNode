//! A number held by one Node has no bitmap posting until a second Node takes
//! the same value. Every query must answer the same either way.

mod common;

use common::{eq, ids, memory, node, ordered, p};
use fastnode::OrderBy;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[test]
fn unique_numbers_need_no_bitmaps() {
    let mut db = memory();
    db.create_many(
        (0..500)
            .map(|i| node(json!({"ts":1_000_000 + i})))
            .collect(),
    )
    .unwrap();
    let blocks = db.stats().unwrap().bitmap_blocks;
    assert!(blocks < 10, "{blocks} bitmap blocks for 500 unique numbers");
    assert_eq!(ids(&mut db, eq("/ts", json!(1_000_123))), vec![124]);
    let within = json!({"op":"in","field":"/ts","values":[1_000_000, 1_000_499, 7]});
    assert_eq!(ids(&mut db, within), vec![1, 500]);
    assert_eq!(
        ids(&mut db, json!({"op":"any","value":1_000_042})),
        vec![43]
    );
    let tail = json!({"op":"range","field":"/ts","gte":1_000_498});
    assert_eq!(ids(&mut db, tail), vec![499, 500]);
}

#[test]
fn second_holder_promotes_and_single_holder_demotes() {
    let mut db = memory();
    let a = db.create(node(json!({"n":7,"tag":"a"}))).unwrap();
    let before = db.stats().unwrap().bitmap_blocks;
    let b = db.create(node(json!({"n":7,"tag":"b"}))).unwrap();
    assert_eq!(ids(&mut db, eq("/n", json!(7))), vec![a, b]);
    db.patch(a, json!({"attrs":{"n":8}})).unwrap();
    assert_eq!(ids(&mut db, eq("/n", json!(7))), vec![b]);
    assert_eq!(ids(&mut db, eq("/n", json!(8))), vec![a]);
    db.patch(a, json!({"attrs":{"n":7}})).unwrap();
    assert_eq!(ids(&mut db, eq("/n", json!(7))), vec![a, b]);
    db.delete(b).unwrap();
    assert_eq!(db.stats().unwrap().bitmap_blocks, before);
    assert_eq!(ids(&mut db, eq("/n", json!(7))), vec![a]);

    db.write(|w| {
        let x = w.create(node(json!({"n":9})))?;
        let y = w.create(node(json!({"n":9})))?;
        let z = w.create(node(json!({"n":9})))?;
        assert_eq!(w.select(&p(eq("/n", json!(9))))?.len(), 3);
        w.delete(x)?;
        w.delete(y)?;
        let left: Vec<u32> = w.select(&p(eq("/n", json!(9))))?.iter().collect();
        assert_eq!(left, vec![z]);
        Ok(())
    })
    .unwrap();
    assert_eq!(ids(&mut db, eq("/n", json!(9))).len(), 1);
    assert_eq!(db.stats().unwrap().bitmap_blocks, before);
}

#[test]
fn numbers_agree_with_scan_through_promotions() {
    let mut db = memory();
    let mut truth: BTreeMap<u32, Value> = BTreeMap::new();
    for i in 0..2600u32 {
        let mut value =
            json!({"big":1_000_000 + i,"arr":[i % 3, i % 1000],"jobs":[{"pay":i % 900}]});
        if i % 97 != 0 {
            value["small"] = json!(i % 1200);
        }
        truth.insert(db.create(node(value.clone())).unwrap(), value);
    }
    for id in (1..=2600u32).step_by(7) {
        let patch = json!({"small":(id * 13) % 1200,"big":2_000_000 + id});
        db.patch(id, json!({"attrs":patch})).unwrap();
        let value = truth.get_mut(&id).unwrap();
        value["small"] = patch["small"].clone();
        value["big"] = patch["big"].clone();
    }
    for id in (3..=2600u32).step_by(11) {
        db.delete(id).unwrap();
        truth.remove(&id);
    }
    let matching = |keep: &dyn Fn(&Value) -> bool| -> Vec<u32> {
        truth
            .iter()
            .filter(|(_, v)| keep(v))
            .map(|(id, _)| *id)
            .collect()
    };
    for target in [
        0u32, 1, 2, 5, 42, 699, 899, 1199, 1_000_010, 2_000_008, 1_000_003,
    ] {
        for field in ["small", "big"] {
            let expected = matching(&|v| v[field] == target);
            assert_eq!(
                ids(&mut db, eq(&format!("/{field}"), json!(target))),
                expected,
                "{field}={target}"
            );
        }
        let anywhere = matching(&|v| {
            v["small"] == target
                || v["big"] == target
                || v["arr"].as_array().unwrap().contains(&json!(target))
                || v["jobs"][0]["pay"] == target
        });
        assert_eq!(
            ids(&mut db, json!({"op":"any","value":target})),
            anywhere,
            "any {target}"
        );
        let paid = matching(&|v| v["jobs"][0]["pay"] == target);
        assert_eq!(ids(&mut db, eq("/jobs/*/pay", json!(target))), paid);
    }
    let within = matching(&|v| [json!(5), json!(42), json!(999)].contains(&v["small"]));
    assert_eq!(
        ids(
            &mut db,
            json!({"op":"in","field":"/small","values":[5,42,999]})
        ),
        within
    );

    let mut by_small: Vec<(u64, u32)> = truth
        .iter()
        .filter_map(|(id, v)| v["small"].as_u64().map(|s| (s, *id)))
        .collect();
    by_small.sort();
    let missing = matching(&|v| v.get("small").is_none());
    let asc: Vec<u32> = by_small
        .iter()
        .map(|(_, id)| *id)
        .chain(missing.clone())
        .collect();
    let desc: Vec<u32> = by_small
        .iter()
        .rev()
        .map(|(_, id)| *id)
        .chain(missing)
        .collect();
    assert_eq!(
        ordered(&mut db, json!({"op":"all"}), OrderBy::asc("/small"), 500),
        asc
    );
    assert_eq!(
        ordered(&mut db, json!({"op":"all"}), OrderBy::desc("/small"), 333),
        desc
    );
}
