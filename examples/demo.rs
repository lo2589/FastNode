use anyhow::{Result, ensure};
use fastnode::{Link, LinkMode, LinkOptions, OrderBy, Predicate, Query, Store};
use serde_json::{Value, json};

fn predicate(value: Value) -> Result<Predicate> {
    Ok(serde_json::from_value(value)?)
}

fn main() -> Result<()> {
    let mut db = Store::open(":memory:")?;
    let imported = db.import(include_bytes!("people.json").as_slice(), 5000)?;
    let query: Query = serde_json::from_str(include_str!("query.json"))?;
    let before = db.query(&query)?;
    ensure!(before.ids == [1]);

    // 上海会场 contains the 2026 event, which contains 小明; 2025 → next → 2026.
    db.write(|w| {
        for (from, relation, to) in [(5, "contains", 4), (4, "contains", 1), (3, "next", 4)] {
            w.link(&Link {
                from,
                relation: relation.into(),
                to,
            })?;
        }
        Ok(())
    })?;
    let traversal: Query = serde_json::from_str(include_str!("traversal.json"))?;
    let place = db.query(&traversal)?;
    ensure!(place.ids == [5]);

    let company = db.select(&predicate(
        json!({"op":"eq","field":"/jobs/*/company","value":"星云科技"}),
    )?)?;
    ensure!(company.iter().collect::<Vec<_>>() == [1, 2]);
    let anywhere = db.select(&Predicate::Any {
        value: json!("星云科技"),
    })?;
    ensure!(anywhere == company);
    let below = db.select(&predicate(
        json!({"op":"traverse","from":{"op":"ids","ids":[5]},"steps":[{"relation":"contains","min":1}]}),
    )?)?;
    ensure!(below.iter().collect::<Vec<_>>() == [1, 4]);

    let mut by_year = Query::new(predicate(json!({"op":"range","field":"/year","gte":2020}))?);
    by_year.order_by = Some(OrderBy::desc("/year"));
    let events = db.query(&by_year)?;
    ensure!(events.ids == [4, 3]);
    let names = db.select(&predicate(
        json!({"op":"range","field":"/name","gte":"Rust","lt":"Rusu"}),
    )?)?;
    ensure!(names.iter().collect::<Vec<_>>() == [3, 4]);

    let event = db.get_with(4, &LinkOptions::default())?;
    let event_ids = db.get_with(
        4,
        &LinkOptions {
            mode: LinkMode::None,
            limit: 100,
        },
    )?;

    db.patch(1, json!({"attrs":{"city":"深圳","skills":["cuda"]}}))?;
    let after = db.query(&query)?;
    ensure!(after.total == 0);
    let changed = db.get(1)?;
    ensure!(db.delete(3)?);
    ensure!(db.get(3)?.is_none());
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "导入":imported,
            "组合查询：person、上海、Rust、20到29岁":before,
            "两跳：小明 ←contains 2026活动 ←contains 会场":place,
            "数组对象通配 /jobs/*/company=星云科技":company.iter().collect::<Vec<_>>(),
            "任意位置值 星云科技":anywhere.iter().collect::<Vec<_>>(),
            "多级：上海会场 contains 的所有下级":below.iter().collect::<Vec<_>>(),
            "排序：year ≥ 2020 按 year 倒序":events.ids,
            "字符串范围：name 以 Rust 开头":names.iter().collect::<Vec<_>>(),
            "拉取 2026活动，links=summary":event,
            "拉取 2026活动，links=none":event_ids,
            "修改后旧条件匹配数":after.total,
            "修改后的Node":changed,
            "最终统计":db.stats()?
        }))?
    );
    Ok(())
}
