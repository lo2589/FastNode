//! Benchmark data and cases, each with an independently computed answer.

use fastnode::{NewNode, OrderBy, Predicate, Query};
use serde_json::{Value, json};

pub const T0: i64 = 1_767_225_600_000; // 2026-01-01 UTC
pub const MINUTE: i64 = 60_000;

pub fn predicate(value: Value) -> Predicate {
    serde_json::from_value(value).unwrap()
}
fn eq(field: &str, value: Value) -> Value {
    json!({"op":"eq","field":field,"value":value})
}
fn city(i: u32) -> &'static str {
    ["上海", "北京", "深圳", "杭州"][(i / 7 % 4) as usize]
}
fn age(i: u32) -> u32 {
    18 + i % 63
}
/// Minute-aligned [start, end), 1 to 30 minutes long.
fn span(i: u32) -> (i64, i64) {
    let start = T0 + i64::from(i) * MINUTE;
    (start, start + (i64::from(i % 30) + 1) * MINUTE)
}

pub fn document(i: u32) -> NewNode {
    let (start, end) = span(i);
    NewNode::new(
        if i.is_multiple_of(5) {
            "person"
        } else {
            "event"
        },
        format!("node {i}，{}", city(i)),
        json!({"person":format!("p{}",i%2500),"city":city(i),"year":2020+i%7,"age":age(i),
               "skills":if i.is_multiple_of(3) {vec!["python","cuda"]} else {vec!["python","rust"]},
               "jobs":[{"company":format!("c{}",i%1000)}],"time":{"start":start,"end":end}}),
    )
}

pub struct Case {
    pub name: &'static str,
    pub predicate: Predicate,
    pub expected: Vec<u32>,
}

fn ids(count: u32, keep: impl Fn(u32) -> bool) -> Vec<u32> {
    (0..count).filter(|i| keep(*i)).map(|i| i + 1).collect()
}

pub fn filters(count: u32) -> Vec<Case> {
    let m = count / 2;
    let minute = |k: u32| T0 + i64::from(k) * MINUTE;
    let (at, window) = (minute(m), (minute(m), minute(m + 60)));
    let case = |name, predicate: Value, expected| Case {
        name,
        predicate: self::predicate(predicate),
        expected,
    };
    vec![
        case(
            "等值 /person=p42",
            eq("/person", json!("p42")),
            ids(count, |i| i % 2500 == 42),
        ),
        case(
            "四属性 AND：person/year/city/@type",
            json!({"op":"and","args":[eq("@type",json!("event")),eq("/city",json!("上海")),eq("/year",json!(2026)),eq("/person",json!("p42"))]}),
            ids(count, |i| {
                i % 2500 == 42 && i % 7 == 6 && city(i) == "上海" && i % 5 != 0
            }),
        ),
        case(
            "标签 + 城市 + 年份 AND",
            json!({"op":"and","args":[eq("/skills",json!("rust")),eq("/city",json!("上海")),eq("/year",json!(2026))]}),
            ids(count, |i| i % 3 != 0 && city(i) == "上海" && i % 7 == 6),
        ),
        case(
            "数字范围 20 ≤ age < 30",
            json!({"op":"range","field":"/age","gte":20,"lt":30}),
            ids(count, |i| (20..30).contains(&age(i))),
        ),
        case(
            "字符串范围 p100 ≤ person < p101",
            json!({"op":"range","field":"/person","gte":"p100","lt":"p101"}),
            ids(count, |i| {
                format!("p{}", i % 2500).as_str() >= "p100"
                    && format!("p{}", i % 2500).as_str() < "p101"
            }),
        ),
        case(
            "OR + NOT",
            json!({"op":"and","args":[{"op":"or","args":[eq("/person",json!("p42")),eq("/person",json!("p43"))]},{"op":"not","arg":eq("/city",json!("北京"))}]}),
            ids(count, |i| matches!(i % 2500, 42 | 43) && city(i) != "北京"),
        ),
        case(
            "字符串前缀 p42",
            json!({"op":"prefix","field":"/person","prefix":"p42"}),
            ids(count, |i| format!("p{}", i % 2500).starts_with("p42")),
        ),
        case(
            "数组对象通配 /jobs/*/company=c42",
            eq("/jobs/*/company", json!("c42")),
            ids(count, |i| i % 1000 == 42),
        ),
        case(
            "任意位置值 p42",
            json!({"op":"any","value":"p42"}),
            ids(count, |i| i % 2500 == 42),
        ),
        case(
            "区间 at：某一分钟在进行的",
            json!({"op":"at","field":"/time","value":at}),
            ids(count, |i| span(i).0 <= at && at < span(i).1),
        ),
        case(
            "区间 overlap：一小时窗口",
            json!({"op":"overlap","field":"/time","start":window.0,"end":window.1}),
            ids(count, |i| span(i).0 < window.1 && span(i).1 > window.0),
        ),
        case(
            "区间 contained_by：一小时窗口",
            json!({"op":"contained_by","field":"/time","start":window.0,"end":window.1}),
            ids(count, |i| span(i).0 >= window.0 && span(i).1 <= window.1),
        ),
        case(
            "person + 区间 overlap（候选探测）",
            json!({"op":"and","args":[eq("/person",json!("p42")),{"op":"overlap","field":"/time","start":T0,"end":minute(m)}]}),
            ids(count, |i| i % 2500 == 42 && i < m),
        ),
        case(
            "多级 next 1..10 跳",
            json!({"op":"traverse","from":eq("/person",json!("p42")),"steps":[{"relation":"next","min":1,"max":10}]}),
            (0..count)
                .filter(|i| i % 2500 == 42)
                .flat_map(|i| (1..=10).map(move |k| (i + k) % count + 1))
                .collect(),
        ),
        case(
            "两跳 next → next，终点城市过滤",
            json!({"op":"traverse","from":eq("/person",json!("p42")),"steps":[{"relation":"next"},{"relation":"next","filter":eq("/city",json!("上海"))}]}),
            (0..count)
                .filter(|i| i % 2500 == 42)
                .map(|i| (i + 2) % count)
                .filter(|i| city(*i) == "上海")
                .map(|i| i + 1)
                .collect(),
        ),
    ]
}

pub struct SortCase {
    pub name: &'static str,
    pub query: Query,
    /// Every matching id in the expected order.
    pub expected: Vec<u32>,
}

pub fn sorts(count: u32) -> Vec<SortCase> {
    let query = |filter: Value, order: OrderBy| {
        let mut query = Query::new(predicate(filter));
        query.order_by = Some(order);
        query
    };
    let sorted = |keep: &dyn Fn(u32) -> bool, key: &dyn Fn(u32) -> (String, u32)| {
        let mut all: Vec<u32> = (0..count).filter(|i| keep(*i)).collect();
        all.sort_by_key(|i| key(*i));
        all.into_iter().map(|i| i + 1).collect::<Vec<_>>()
    };
    vec![
        SortCase {
            name: "@type=event 按 age 升序（有序行扫，候选密集）",
            query: query(eq("@type", json!("event")), OrderBy::asc("/age")),
            expected: sorted(&|i| i % 5 != 0, &|i| (format!("{:03}", age(i)), i)),
        },
        SortCase {
            name: "city=上海 按 time.start 降序（取值多，有序行扫）",
            query: query(eq("/city", json!("上海")), OrderBy::desc("/time/start")),
            expected: ids(count, |i| city(i) == "上海")
                .into_iter()
                .rev()
                .collect(),
        },
        SortCase {
            name: "skills=rust 按 person 字符串升序",
            query: query(eq("/skills", json!("rust")), OrderBy::asc("/person")),
            expected: sorted(&|i| i % 3 != 0, &|i| (format!("p{}", i % 2500), i)),
        },
        SortCase {
            name: "person=p42 按 time.start 降序（候选少，逐个取键）",
            query: query(eq("/person", json!("p42")), OrderBy::desc("/time/start")),
            expected: ids(count, |i| i % 2500 == 42).into_iter().rev().collect(),
        },
    ]
}
