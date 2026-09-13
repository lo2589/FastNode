//! Indexed answers compared with independent scans of the same data.

mod common;

use common::{eq, ids, memory, node, ordered};
use fastnode::{OrderBy, Store};
use serde_json::{Value, json};
use std::collections::BTreeMap;

type Truth = BTreeMap<u32, Value>;

#[test]
fn filters_agree_with_scan_after_mutations() {
    let mut db = memory();
    let mut truth = Truth::new();
    for i in 0..300 {
        let skills = if i % 7 == 0 {
            vec!["rust", "python"]
        } else {
            vec!["python"]
        };
        let value = json!({"city":(["上海","北京","深圳"][i%3]),"age":i%71,"skills":skills,"jobs":[{"company":format!("c{}",i%5)}]});
        truth.insert(db.create(node(value.clone())).unwrap(), value);
    }
    for id in 1..=300 {
        if id % 11 == 0 {
            db.delete(id).unwrap();
            truth.remove(&id);
        } else if id % 13 == 0 {
            let patch = json!({"city":"深圳","age":42,"jobs":[{"company":"c9"}]});
            db.patch(id, json!({"attrs":patch})).unwrap();
            let value = truth.get_mut(&id).unwrap();
            fastnode_merge(value, &patch);
        }
    }
    for city in ["上海", "北京", "深圳"] {
        for lower in [0, 20, 40, 60] {
            let query = json!({"op":"and","args":[eq("/city",json!(city)),{"op":"range","field":"/age","gte":lower,"lt":lower+10},{"op":"not","arg":eq("/skills",json!("rust"))}]});
            let expected = matching(&truth, |v| {
                v["city"] == city
                    && (lower..lower + 10).contains(&v["age"].as_i64().unwrap())
                    && !v["skills"].as_array().unwrap().contains(&json!("rust"))
            });
            assert_eq!(ids(&mut db, query), expected);
        }
    }
    for company in ["c0", "c3", "c9"] {
        let expected = matching(&truth, |v| v["jobs"][0]["company"] == company);
        assert_eq!(
            ids(&mut db, eq("/jobs/*/company", json!(company))),
            expected
        );
        assert_eq!(ids(&mut db, json!({"op":"any","value":company})), expected);
    }
}

#[test]
fn every_sort_strategy_agrees_with_sorting_in_memory() {
    let mut db = memory();
    let mut truth = Truth::new();
    for i in 0..6000u32 {
        let mixed = match i % 5 {
            0 => json!(format!("m{}", i % 40)),
            1 => Value::Null,
            _ => json!(i % 900),
        };
        let value = json!({"grade":i % 7,"ts":(i * 7919) % 6000,"name":format!("n{:03}",(i * 31) % 500),"mixed":mixed});
        truth.insert(db.create(node(value.clone())).unwrap(), value);
    }
    for id in (1..=6000).step_by(97) {
        db.delete(id).unwrap();
        truth.remove(&id);
    }
    for id in (5..=6000).step_by(89) {
        if truth.contains_key(&id) {
            db.patch(id, json!({"attrs":{"grade":"top","mixed":null}}))
                .unwrap();
            let value = truth.get_mut(&id).unwrap();
            value["grade"] = json!("top");
            value["mixed"] = Value::Null;
        }
    }
    // grade=3 stays below the probe limit; its complement streams.
    let small = eq("/grade", json!(3));
    let large = json!({"op":"not","arg":small.clone()});
    for field in ["grade", "ts", "name", "mixed"] {
        let is_three: fn(&Value) -> bool = |v| v["grade"] == 3;
        let not_three: fn(&Value) -> bool = |v| v["grade"] != 3;
        for (candidates, filter) in [(small.clone(), is_three), (large.clone(), not_three)] {
            let members: Vec<u32> = matching(&truth, filter);
            for desc in [false, true] {
                let order = if desc {
                    OrderBy::desc(format!("/{field}"))
                } else {
                    OrderBy::asc(format!("/{field}"))
                };
                let expected = sorted(&truth, &members, field, desc);
                assert_eq!(
                    ordered(&mut db, candidates.clone(), order, 700),
                    expected,
                    "{field} desc={desc}"
                );
            }
        }
    }
}

#[test]
fn intervals_agree_with_scan() {
    let mut db: Store = memory();
    let mut truth = Truth::new();
    for i in 0..800i64 {
        let span = |s: i64, kind: i64| match kind {
            0 => json!({"start":s,"end":s + 5}),
            1 => json!({"start":s,"end":s}),
            2 => json!({"start":s}),
            _ => json!({"start":s,"end":s + 17}),
        };
        let value = json!({"span":span(i % 97, i % 4),"jobs":[{"t":span((i * 13) % 101, (i + 1) % 4)},{"t":span((i * 7) % 89, (i + 2) % 4)}]});
        truth.insert(db.create(node(value.clone())).unwrap(), value);
    }
    let bounds = |v: &Value| (v["start"].as_i64().unwrap(), v["end"].as_i64());
    type Rule = fn(i64, Option<i64>, i64, i64) -> bool;
    let rules: [(&str, Rule); 4] = [
        ("at", |s, e, a, _| {
            s <= a && (e.is_none_or(|e| a < e) || (e == Some(s) && s == a))
        }),
        ("overlap", |s, e, a, b| {
            s < b && (e.is_none_or(|e| e > a) || (e == Some(s) && s >= a))
        }),
        ("contains", |s, e, a, b| s <= a && e.is_none_or(|e| e >= b)),
        ("contained_by", |s, e, a, b| {
            s >= a && e.is_some_and(|e| e <= b && (e > s || s < b))
        }),
    ];
    for (name, rule) in rules {
        for (a, b) in [
            (0, 0),
            (3, 9),
            (10, 10),
            (20, 60),
            (50, 51),
            (88, 120),
            (-5, 2),
        ] {
            let query = |field: &str| match name {
                "at" => json!({"op":"at","field":field,"value":a}),
                _ => json!({"op":name,"field":field,"start":a,"end":b}),
            };
            let hit = |v: &Value| {
                let (s, e) = bounds(v);
                rule(s, e, a, b)
            };
            let direct = matching(&truth, |v| hit(&v["span"]));
            assert_eq!(ids(&mut db, query("/span")), direct, "{name} {a} {b}");
            let nested = matching(&truth, |v| {
                v["jobs"].as_array().unwrap().iter().any(|j| hit(&j["t"]))
            });
            assert_eq!(
                ids(&mut db, query("/jobs/*/t")),
                nested,
                "{name} {a} {b} nested"
            );
        }
    }
}

fn matching(truth: &Truth, keep: impl Fn(&Value) -> bool) -> Vec<u32> {
    truth
        .iter()
        .filter(|(_, v)| keep(v))
        .map(|(id, _)| *id)
        .collect()
}

// Numbers, then strings, then no value; descending reverses the first two.
fn sorted(truth: &Truth, members: &[u32], field: &str, desc: bool) -> Vec<u32> {
    let (mut numbers, mut strings, mut missing) = (Vec::new(), Vec::new(), Vec::new());
    for id in members {
        match &truth[id][field] {
            Value::Number(n) => numbers.push((n.as_i64().unwrap(), *id)),
            Value::String(s) => strings.push((s.clone(), *id)),
            _ => missing.push(*id),
        }
    }
    numbers.sort();
    strings.sort();
    missing.sort();
    let numbers = numbers.into_iter().map(|(_, id)| id);
    let strings = strings.into_iter().map(|(_, id)| id);
    let head: Vec<u32> = if desc {
        strings.rev().chain(numbers.rev()).collect()
    } else {
        numbers.chain(strings).collect()
    };
    head.into_iter().chain(missing).collect()
}

fn fastnode_merge(target: &mut Value, patch: &Value) {
    for (key, value) in patch.as_object().unwrap() {
        target[key] = value.clone();
    }
}
