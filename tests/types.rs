mod common;

use common::memory;
use fastnode::types::{self, EventNode, Span, StateNode, TreeNode};
use fastnode::{LinkOptions, NewNode};
use serde_json::json;

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
