//! Derived Node types on top of the core: tree, event and state.

use anyhow::{Result, ensure};
use fastnode::types::{EventNode, Span, StateNode, TreeNode};
use fastnode::{NewNode, Store};
use serde_json::json;

const DAY: i64 = 1_789_228_800_000; // 2026-09-13 00:00 +08:00
const HOUR: i64 = 3_600_000;

fn main() -> Result<()> {
    let mut db = Store::open(":memory:")?;

    // tree: parent / children / next / before lifted to first-level fields.
    let docs = TreeNode::create(&mut db, "文档", "项目文档根目录", None)?;
    let guide = TreeNode::create(&mut db, "指南", "使用指南", Some(docs))?;
    let api = TreeNode::create(&mut db, "API", "接口说明", Some(docs))?;
    let query = TreeNode::create(&mut db, "查询", "查询协议", Some(api))?;
    TreeNode::set_next(&mut db, guide, api)?;
    let root = TreeNode::read(&mut db, docs)?;
    let below: Vec<u32> = TreeNode::descendants(&mut db, docs)?.iter().collect();
    ensure!(below == [guide, api, query]);

    // event: "今天吃饭，然后逛商场，然后睡觉" — rough time, exact order.
    let today = Span::new(DAY, Some(DAY + 24 * HOUR)).with_text("今天");
    let eat = EventNode::create(&mut db, "吃饭", "今天吃了饭", today.clone())?;
    let shop = EventNode::create(&mut db, "逛商场", "然后逛商场", today.clone())?;
    let sleep = EventNode::create(&mut db, "睡觉", "然后睡觉", today)?;
    EventNode::then(&mut db, eat, shop)?;
    EventNode::then(&mut db, shop, sleep)?;
    let during_today = EventNode::during(&mut db, DAY, DAY + 24 * HOUR, 10)?;
    let after_eating = EventNode::after(&mut db, eat)?;
    ensure!(after_eating.iter().map(|e| e.id).collect::<Vec<_>>() == [shop, sleep]);

    // state: a value that changes over time, answered as of any instant.
    let person = db.create(NewNode::new("person", "小明", json!({"name":"小明"})))?;
    StateNode::set(
        &mut db,
        person,
        "city",
        json!("上海"),
        DAY - 400 * 24 * HOUR,
        "住在上海",
    )?;
    StateNode::set(
        &mut db,
        person,
        "city",
        json!("北京"),
        DAY - 30 * 24 * HOUR,
        "搬到北京",
    )?;
    let a_year_ago = StateNode::at(&mut db, person, "city", DAY - 365 * 24 * HOUR)?;
    let now = StateNode::at(&mut db, person, "city", DAY)?;
    ensure!(now.as_ref().map(|s| s.value.clone()) == Some(json!("北京")));

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "tree：文档根目录":root,
            "tree：文档下所有节点":below,
            "event：今天发生的事，按开始时间":during_today,
            "event：吃饭之后":after_eating,
            "state：一年前住哪":a_year_ago,
            "state：现在住哪":now,
            "state：city 历史":StateNode::history(&mut db, person, "city")?,
        }))?
    );
    Ok(())
}
