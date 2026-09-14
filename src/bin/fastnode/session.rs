//! rpc transactions: {"op":"begin"}, then any reads and writes on one write
//! transaction, then {"op":"commit"} or {"op":"rollback"}. After a failed
//! request only rollback is accepted; input ending mid-transaction rolls back.

use crate::dispatch::{link_options, text};
use crate::mutation::Mutation;
use anyhow::{Context, Result, anyhow, bail};
use fastnode::{NewNode, Query, Store, Write};
use serde_json::{Value, json};
use std::io::{self, Write as IoWrite};

pub fn reply(out: &mut impl IoWrite, result: Result<Value>) -> Result<()> {
    let output = match result {
        Ok(result) => json!({"ok":true,"result":result}),
        Err(error) => json!({"ok":false,"error":format!("{error:#}")}),
    };
    writeln!(out, "{output}")?;
    out.flush()?;
    Ok(())
}

enum End {
    Commit,
    Rollback,
    Eof,
}

pub fn run<I>(store: &mut Store, lines: &mut I, out: &mut impl IoWrite) -> Result<()>
where
    I: Iterator<Item = io::Result<String>>,
{
    let mut end = End::Eof;
    let result = store.write(|w| {
        let mut failed = false;
        for line in lines.by_ref() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let request: Value = match serde_json::from_str(&line) {
                Ok(request) => request,
                Err(error) => {
                    failed = true;
                    reply(out, Err(error.into()))?;
                    continue;
                }
            };
            match request.get("op").and_then(Value::as_str) {
                Some("rollback") => {
                    end = End::Rollback;
                    bail!("rollback requested");
                }
                Some("commit") if !failed => {
                    end = End::Commit;
                    return Ok(());
                }
                Some("begin") => reply(out, Err(anyhow!("already inside a transaction")))?,
                _ if failed => reply(
                    out,
                    Err(anyhow!(
                        "an earlier request failed; only rollback is accepted"
                    )),
                )?,
                _ => {
                    let result = in_transaction(w, &request);
                    failed = result.is_err();
                    reply(out, result)?;
                }
            }
        }
        bail!("input ended inside a transaction")
    });
    match (end, result) {
        (End::Commit, Ok(())) => reply(out, Ok(json!({"committed": true}))),
        (End::Commit, Err(error)) => reply(out, Err(error)),
        (End::Rollback, _) => reply(out, Ok(json!({"rolled_back": true}))),
        (End::Eof, _) => Ok(()),
    }
}

fn in_transaction(w: &mut Write<'_>, request: &Value) -> Result<Value> {
    let op = request
        .get("op")
        .and_then(Value::as_str)
        .context("request requires op")?;
    let id = || -> Result<u32> {
        let id = request.get("id").and_then(Value::as_u64);
        Ok(u32::try_from(id.context("id must be a u32")?)?)
    };
    let field = |name: &str| {
        request
            .get(name)
            .with_context(|| format!("{name} is required"))
    };
    Ok(match op {
        "get" => serde_json::to_value(w.get_with(id()?, &link_options(request)?)?)?,
        "neighbors" => {
            let direction = request
                .get("direction")
                .map(|d| serde_json::from_value(d.clone()))
                .transpose()?;
            let (links, more) = w.neighbors(
                id()?,
                text(request, "relation")?,
                direction.unwrap_or_default(),
                &link_options(request)?,
            )?;
            json!({"links": links, "more": more})
        }
        "query" => {
            let query: Query = serde_json::from_value(field("query")?.clone())?;
            serde_json::to_value(w.query(&query)?)?
        }
        "now" => json!({"now": w.now()?}),
        "create_many" => {
            let nodes: Vec<NewNode> = serde_json::from_value(field("nodes")?.clone())?;
            let ids = nodes
                .into_iter()
                .map(|n| w.create(n))
                .collect::<Result<Vec<_>>>()?;
            json!({"ids": ids})
        }
        "batch" => {
            let mutations: Vec<Mutation> = serde_json::from_value(field("ops")?.clone())?;
            let results = mutations
                .into_iter()
                .map(|m| m.run(w))
                .collect::<Result<Vec<_>>>()?;
            json!({"results": results})
        }
        "view" | "links" | "stats" | "define_index" | "define_type" | "typedefs" | "set_policy" => {
            bail!("{op} is not available inside a transaction")
        }
        _ => serde_json::from_value::<Mutation>(request.clone())?.run(w)?,
    })
}
