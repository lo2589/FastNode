mod common;

use common::{Temp, memory};
use fastnode::types::{self, CodeNode, EventNode, RelationshipNode, Span, StateNode, TreeNode, TypeDef};
use fastnode::{LinkOptions, NewNode, Store};
use serde_json::{Value, json};

const DAY: i64 = 1_789_228_800_000;
const HOUR: i64 = 3_600_000;

#[test]
fn tree_lifts_parent_children_and_sibling_order() {
    let mut db = memory();
    let root = TreeNode::create(&mut db, "root", "the root", None).unwrap();
    let a = TreeNode::create(&mut db, "a", "first child", Some(root)).unwrap();
    let b = TreeNode::create(&mut db, "b", "second child", Some(root)).unwrap();
    let c = TreeNode::create(&mut db, "c", "grandchild", Some(a)).unwrap();
    assert!(TreeNode::set_next(&mut db, a, b).unwrap());

    let node = TreeNode::read(&mut db, root).unwrap().unwrap();
    assert_eq!(node.name.as_deref(), Some("root"));
    assert!(node.parent.is_none());
    let children: Vec<_> = node
        .children
        .iter()
        .map(|l| (l.id, l.summary.clone().unwrap()))
        .collect();
    assert_eq!(
        children,
        vec![(a, "first child".into()), (b, "second child".into())]
    );

    let first = TreeNode::read(&mut db, a).unwrap().unwrap();
    assert_eq!(first.parent.as_ref().map(|l| l.id), Some(root));
    assert_eq!(first.next.as_ref().map(|l| l.id), Some(b));
    let second = TreeNode::read(&mut db, b).unwrap().unwrap();
    assert_eq!(second.before.as_ref().map(|l| l.id), Some(a));

    let below: Vec<u32> = TreeNode::descendants(&mut db, root)
        .unwrap()
        .iter()
        .collect();
    assert_eq!(below, vec![a, b, c]);
    let above: Vec<u32> = TreeNode::ancestors(&mut db, c).unwrap().iter().collect();
    assert_eq!(above, vec![root, a]);

    // The same fields through the generic view, flat as JSON consumers see them.
    let view = types::view(&mut db, "tree", a, &LinkOptions::default())
        .unwrap()
        .unwrap();
    let flat = serde_json::to_value(&view).unwrap();
    assert_eq!(
        (
            flat["type"].clone(),
            flat["name"].clone(),
            flat["next"]["id"].clone()
        ),
        (json!("tree"), json!("a"), json!(b))
    );
    assert!(types::view(&mut db, "event", a, &LinkOptions::default()).is_err());
    assert!(types::view(&mut db, "nope", a, &LinkOptions::default()).is_err());
}

#[test]
fn events_have_rough_time_and_exact_order() {
    let mut db = memory();
    let today = Span::new(DAY, Some(DAY + 24 * HOUR)).with_text("今天");
    let eat = EventNode::create(&mut db, "吃饭", "今天吃了饭", today.clone()).unwrap();
    let shop = EventNode::create(&mut db, "逛商场", "然后逛了商场", today.clone()).unwrap();
    let sleep = EventNode::create(&mut db, "睡觉", "最后睡觉", today).unwrap();
    EventNode::then(&mut db, eat, shop).unwrap();
    EventNode::then(&mut db, shop, sleep).unwrap();

    let during: Vec<_> = EventNode::during(&mut db, DAY, DAY + 24 * HOUR, 10)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(during, vec![eat, shop, sleep]);
    let middle = EventNode::read(&mut db, shop).unwrap().unwrap();
    assert_eq!(middle.what.as_deref(), Some("逛商场"));
    assert_eq!(middle.next.as_ref().map(|l| l.id), Some(sleep));
    assert_eq!(
        middle
            .before
            .as_ref()
            .and_then(|l| l.summary.clone())
            .as_deref(),
        Some("今天吃了饭")
    );
    let later: Vec<_> = EventNode::after(&mut db, eat)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(later, vec![shop, sleep]);

    let noon = Span::new(DAY + 12 * HOUR, Some(DAY + 13 * HOUR)).with_text("中午12点");
    db.patch(eat, json!({"attrs":{"time":noon}})).unwrap();
    let evening: Vec<_> = EventNode::during(&mut db, DAY + 18 * HOUR, DAY + 24 * HOUR, 10)
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(evening, vec![shop, sleep]);
    assert_eq!(
        EventNode::read(&mut db, eat)
            .unwrap()
            .unwrap()
            .time
            .unwrap()
            .text
            .as_deref(),
        Some("中午12点")
    );
}

#[test]
fn state_history_splits_and_answers_as_of() {
    let mut db = memory();
    let person = db
        .create(NewNode::new("person", "小明", json!({"name":"小明"})))
        .unwrap();
    let shanghai = StateNode::set(&mut db, person, "city", json!("上海"), 100, "住上海").unwrap();
    StateNode::set(&mut db, person, "city", json!("北京"), 200, "搬去北京").unwrap();
    StateNode::set(&mut db, person, "city", json!("深圳"), 150, "中间在深圳").unwrap();
    StateNode::set(&mut db, person, "mood", json!("好"), 0, "心情").unwrap();

    let city_at = |db: &mut fastnode::Store, t| {
        StateNode::at(db, person, "city", t)
            .unwrap()
            .map(|s| s.value)
    };
    assert_eq!(city_at(&mut db, 50), None);
    assert_eq!(city_at(&mut db, 120), Some(json!("上海")));
    assert_eq!(city_at(&mut db, 150), Some(json!("深圳")));
    assert_eq!(city_at(&mut db, 199), Some(json!("深圳")));
    assert_eq!(city_at(&mut db, 10_000), Some(json!("北京")));

    let history = StateNode::history(&mut db, person, "city").unwrap();
    let spans: Vec<_> = history
        .iter()
        .map(|s| (s.value.clone(), s.valid.as_ref().map(|v| (v.start, v.end))))
        .collect();
    assert_eq!(
        spans,
        vec![
            (json!("上海"), Some((100, Some(150)))),
            (json!("深圳"), Some((150, Some(200)))),
            (json!("北京"), Some((200, None))),
        ]
    );
    assert_eq!(history[0].id, shanghai);
    assert_eq!(history[0].subject.as_ref().map(|l| l.id), Some(person));

    let same = StateNode::set(&mut db, person, "city", json!("杭州"), 150, "改成杭州").unwrap();
    assert_eq!(
        StateNode::history(&mut db, person, "city").unwrap().len(),
        3
    );
    assert_eq!(city_at(&mut db, 160), Some(json!("杭州")));
    assert_eq!(
        StateNode::at(&mut db, person, "city", 160)
            .unwrap()
            .unwrap()
            .id,
        same
    );
    assert_eq!(
        StateNode::history(&mut db, person, "mood").unwrap().len(),
        1
    );
}

#[test]
fn code_lifts_symbol_fields_and_call_links() {
    let mut db = memory();
    let main_fn = CodeNode::create(
        &mut db,
        "main main() (src/main.rs:1)",
        json!({"symbol":"main","file":"src/main.rs","line_start":1,"symbol_type":"function"}),
    )
    .unwrap();
    let helper = CodeNode::create(
        &mut db,
        "helper helper() (src/lib.rs:9)",
        json!({"symbol":"helper","file":"src/lib.rs","line_start":9,"symbol_type":"function"}),
    )
    .unwrap();
    db.link(&fastnode::Link { from: main_fn, relation: "call".into(), to: helper }).unwrap();

    let node = CodeNode::read(&mut db, main_fn).unwrap().unwrap();
    assert_eq!(node.symbol.as_deref(), Some("main"));
    assert_eq!(node.file.as_deref(), Some("src/main.rs"));
    assert_eq!(node.line_start, Some(1));
    assert_eq!(node.calls.iter().map(|l| l.id).collect::<Vec<_>>(), vec![helper]);

    let callee = CodeNode::read(&mut db, helper).unwrap().unwrap();
    assert_eq!(callee.called_by.iter().map(|l| l.id).collect::<Vec<_>>(), vec![main_fn]);

    let view = types::view(&mut db, "code", main_fn, &LinkOptions::default())
        .unwrap()
        .unwrap();
    let flat = serde_json::to_value(&view).unwrap();
    assert_eq!(
        (flat["type"].clone(), flat["symbol_type"].clone(), flat["calls"][0]["id"].clone()),
        (json!("code"), json!("function"), json!(helper))
    );
    assert!(types::view(&mut db, "tree", main_fn, &LinkOptions::default()).is_err());
}

#[test]
fn runtime_defined_types_view_without_recompile() {
    let tmp = Temp::new();
    let mut db = Store::open(&tmp.0).unwrap();
    let def: TypeDef = serde_json::from_value(json!({
        "kind": "service",
        "fields": [
            {"name": "name", "attr": "/name"},
            {"name": "endpoints", "link": "exposes", "direction": "out", "many": true},
            {"name": "owner", "link": "owned_by", "direction": "in"}
        ]
    }))
    .unwrap();
    types::define_type(&mut db, &def).unwrap();
    types::define_type(&mut db, &def).unwrap(); // identical re-define is a no-op

    let svc = db.create(NewNode::new("service", "网关", json!({"name":"gateway"}))).unwrap();
    let ep = db.create(NewNode::new("endpoint", "健康检查", json!({}))).unwrap();
    db.link(&fastnode::Link { from: svc, relation: "exposes".into(), to: ep }).unwrap();

    let flat = serde_json::to_value(
        types::view(&mut db, "service", svc, &LinkOptions::default())
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (flat["name"].clone(), flat["endpoints"][0]["id"].clone(), flat["owner"].clone()),
        (json!("gateway"), json!(ep), Value::Null)
    );

    drop(db);
    let mut db = Store::open(&tmp.0).unwrap();
    assert_eq!(types::type_defs(&mut db).unwrap(), vec![def.clone()]);
    assert!(
        types::view(&mut db, "service", svc, &LinkOptions::default())
            .unwrap()
            .is_some()
    );
    assert!(types::view(&mut db, "nope", svc, &LinkOptions::default()).is_err());

    let mut conflicting = def.clone();
    conflicting.fields.pop();
    assert!(types::define_type(&mut db, &conflicting).is_err());
    for bad in [
        json!({"kind":"x","fields":[{"name":"a","attr":"name"}]}),        // attr 缺前导 /
        json!({"kind":"x","fields":[{"name":"a"}]}),                      // attr/link 都没有
        json!({"kind":"x","fields":[{"name":"a","attr":"/a","link":"b"}]}), // 两个都有
        json!({"kind":"x","fields":[{"name":"a","attr":"/a"},{"name":"a","attr":"/b"}]}), // 重名
        json!({"kind":"tree","fields":[{"name":"a","attr":"/a"}]}),       // 内置类型不可重定义
    ] {
        let def: TypeDef = serde_json::from_value(bad).unwrap();
        assert!(types::define_type(&mut db, &def).is_err(), "{def:?}");
    }
}

#[test]
fn relationship_lifts_endpoints_and_edge_metadata() {
    let mut db = memory();
    let a = CodeNode::create(&mut db, "a() (a.rs:1)", json!({"symbol":"a"})).unwrap();
    let b = CodeNode::create(&mut db, "b() (b.rs:2)", json!({"symbol":"b"})).unwrap();
    let edge = RelationshipNode::create(
        &mut db,
        "a -[call]-> b @42",
        json!({"edge_type":"call","call_line":42,"is_conditional":true,"resolution":"exact"}),
        a,
        b,
    )
    .unwrap();

    let node = RelationshipNode::read(&mut db, edge).unwrap().unwrap();
    assert_eq!(node.from.as_ref().map(|l| l.id), Some(a));
    assert_eq!(node.to.as_ref().map(|l| l.id), Some(b));
    assert_eq!(node.edge_type.as_deref(), Some("call"));
    assert_eq!(node.call_line, Some(42));
    assert_eq!(node.is_conditional, Some(true));

    let flat = serde_json::to_value(
        types::view(&mut db, "relationship", edge, &LinkOptions::default())
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (flat["type"].clone(), flat["resolution"].clone(), flat["from"]["id"].clone()),
        (json!("relationship"), json!("exact"), json!(a))
    );
}
