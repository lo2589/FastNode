mod cases;

use anyhow::{Result, ensure};
use fastnode::{Link, LinkMode, LinkOptions, NewNode, Query, Store};
use serde_json::{Value, json};
use std::{env, fs, path::Path, time::Instant};

fn percentiles(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    let at = |p: f64| samples[((samples.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    json!({"samples":samples.len(),"p50_ms":at(0.5),"p95_ms":at(0.95),"min_ms":samples[0],"max_ms":samples[samples.len()-1]})
}

/// 3 warmups, then 50 timed calls.
fn timed(mut call: impl FnMut() -> Result<()>) -> Result<Value> {
    for _ in 0..3 {
        call()?;
    }
    let mut samples = Vec::new();
    for _ in 0..50 {
        let started = Instant::now();
        call()?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(percentiles(samples))
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    let path = args
        .first()
        .map(String::as_str)
        .unwrap_or("data/million.db");
    let count: u32 = args
        .get(1)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(1_000_000);
    ensure!(count >= 10_000, "benchmark requires at least 10000 nodes");
    ensure!(
        !Path::new(path).exists(),
        "benchmark creates a fresh database; choose a new path"
    );
    if let Some(parent) = Path::new(path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut db = Store::open(path)?;
    let started = Instant::now();
    for start in (0..count).step_by(5000) {
        db.create_many(
            (start..(start + 5000).min(count))
                .map(cases::document)
                .collect(),
        )?;
    }
    let import_seconds = started.elapsed().as_secs_f64();
    db.checkpoint()?;
    let node_database_bytes = fs::metadata(path)?.len();
    let started = Instant::now();
    for start in (1..=count).step_by(5000) {
        db.write(|w| {
            for from in start..(start + 5000).min(count + 1) {
                let to = if from == count { 1 } else { from + 1 };
                w.link(&Link {
                    from,
                    relation: "next".into(),
                    to,
                })?;
            }
            Ok(())
        })?;
    }
    let link_seconds = started.elapsed().as_secs_f64();
    db.checkpoint()?;
    eprintln!("created {count} nodes in {import_seconds:.2}s, links in {link_seconds:.2}s");

    let filters = run_filters(&mut db, count)?;
    let sorts = run_sorts(&mut db, count)?;
    let gets = run_gets(&mut db, count)?;
    let crud = run_crud(&mut db)?;
    let stats = db.stats()?;
    ensure!(
        stats.nodes == u64::from(count)
            && stats.links == u64::from(count)
            && stats.intervals == u64::from(count),
        "final dataset size mismatch"
    );
    db.checkpoint()?;
    drop(db);
    let started = Instant::now();
    let mut db = Store::open(path)?;
    let reopen_ms = started.elapsed().as_secs_f64() * 1000.0;
    let reopen_case = &cases::filters(count)[1];
    let started = Instant::now();
    ensure!(
        db.select(&reopen_case.predicate)?.len() == reopen_case.expected.len() as u64,
        "reopen mismatch"
    );
    let reopen_query_ms = started.elapsed().as_secs_f64() * 1000.0;
    let report = json!({
        "nodes":count,"links":count,"batch_size":5000,"profile":"release","synchronous":"FULL","journal_mode":"WAL",
        "import_seconds":import_seconds,"nodes_per_second":f64::from(count)/import_seconds,
        "link_seconds":link_seconds,"node_database_bytes":node_database_bytes,"database_bytes":fs::metadata(path)?.len(),"stats":stats,
        "query_measurement":"same-process library calls, 3 warmups + 50 samples; full matching bitmap computed, first 100 returned; payload uses links=none",
        "reopen_ms":reopen_ms,"reopen_query_ms":reopen_query_ms,
        "queries":filters,"sorts":sorts,"gets":gets,"single_transaction_crud":crud,
        "correctness":"Every filter compared with an independently generated id list; every sorted page and the page after a 100000-row cursor compared with an independently sorted list; counts, updates, deletes and reopen checked."
    });
    fs::create_dir_all("reports")?;
    let report_path = format!("reports/benchmark-{count}.json");
    fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    eprintln!("report: {report_path}");
    Ok(())
}

fn run_filters(db: &mut Store, count: u32) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    for case in cases::filters(count) {
        let mut expected = case.expected.clone();
        expected.sort_unstable();
        expected.dedup();
        ensure!(
            db.select(&case.predicate)?
                .iter()
                .eq(expected.iter().copied()),
            "incorrect result: {}",
            case.name
        );
        for include_data in [false, true] {
            let mut query = Query::new(case.predicate.clone());
            query.include_data = include_data;
            query.links = LinkMode::None;
            let first_page: Vec<u32> = expected.iter().copied().take(100).collect();
            let timing = timed(|| {
                let result = db.query(&query)?;
                ensure!(
                    result.total == expected.len() as u64 && result.ids == first_page,
                    "incorrect page"
                );
                Ok(())
            })?;
            eprintln!(
                "{} payload={include_data}: p50={:.4}ms",
                case.name,
                timing["p50_ms"].as_f64().unwrap()
            );
            rows.push(json!({"name":case.name,"include_data":include_data,"matched":expected.len(),"timing":timing}));
        }
    }
    Ok(rows)
}

fn run_sorts(db: &mut Store, count: u32) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    for case in cases::sorts(count) {
        let deep = (case.expected.len() / 2).min(100_000) / 100 * 100;
        let mut query = case.query.clone();
        query.limit = 10_000.min(deep.max(100));
        let mut seen = Vec::new();
        while seen.len() < deep {
            let page = db.query(&query)?;
            seen.extend(page.ids);
            query.cursor = page.next_cursor;
        }
        ensure!(
            seen[..] == case.expected[..seen.len()],
            "incorrect ordered pages: {}",
            case.name
        );
        let deep_cursor = query.cursor.clone();
        let mut row =
            json!({"name":case.name,"matched":case.expected.len(),"deep_offset":seen.len()});
        for (label, cursor, offset) in [
            ("first_page", None, 0),
            ("deep_page", deep_cursor, seen.len()),
        ] {
            query.limit = 100;
            query.cursor = cursor;
            let page: Vec<u32> = case
                .expected
                .iter()
                .skip(offset)
                .take(100)
                .copied()
                .collect();
            row[label] = timed(|| {
                ensure!(db.query(&query)?.ids == page, "incorrect sorted page");
                Ok(())
            })?;
        }
        eprintln!(
            "{}: first p50={:.4}ms deep p50={:.4}ms",
            case.name,
            row["first_page"]["p50_ms"].as_f64().unwrap(),
            row["deep_page"]["p50_ms"].as_f64().unwrap()
        );
        rows.push(row);
    }
    Ok(rows)
}

fn run_gets(db: &mut Store, count: u32) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    for mode in [LinkMode::None, LinkMode::Summary, LinkMode::Full] {
        let options = LinkOptions { mode, limit: 100 };
        let mut i = 0u32;
        let timing = timed(|| {
            i += 1;
            let id = 1 + (i * 18_869) % count;
            let links = db
                .get_with(id, &options)?
                .expect("node exists")
                .links
                .expect("links");
            ensure!(
                links.out.len() == 1
                    && links.incoming.len() == 1
                    && links.out[0].id == id % count + 1,
                "incorrect links"
            );
            ensure!(
                (mode == LinkMode::None) == links.out[0].summary.is_none(),
                "incorrect link mode"
            );
            Ok(())
        })?;
        rows.push(json!({"links":mode,"timing":timing}));
    }
    Ok(rows)
}

fn run_crud(db: &mut Store) -> Result<Value> {
    let (mut created, mut create, mut patch, mut delete) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let ms = |started: Instant| started.elapsed().as_secs_f64() * 1000.0;
    for _ in 0..100 {
        let started = Instant::now();
        let node = NewNode::new(
            "bench",
            "temporary benchmark node",
            json!({"bench":"temporary","age":21,"skills":["rust"],"time":{"start":0,"end":10}}),
        );
        created.push(db.create(node)?);
        create.push(ms(started));
    }
    for id in &created {
        let started = Instant::now();
        db.patch(
            *id,
            json!({"attrs":{"age":31,"skills":["cuda"],"time":{"end":20}}}),
        )?;
        patch.push(ms(started));
    }
    for id in &created {
        let started = Instant::now();
        db.delete(*id)?;
        delete.push(ms(started));
    }
    ensure!(
        db.select(&cases::predicate(
            json!({"op":"eq","field":"/bench","value":"temporary"})
        ))?
        .is_empty(),
        "stale index after delete"
    );
    Ok(
        json!({"create":percentiles(create),"patch":percentiles(patch),"delete":percentiles(delete)}),
    )
}
