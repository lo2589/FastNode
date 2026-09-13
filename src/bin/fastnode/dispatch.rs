use crate::mutation::Mutation;
use anyhow::{Context, Result};
use fastnode::{LinkOptions, NewNode, Query, Store, types};
use serde_json::{Value, json};

pub fn dispatch(store: &mut Store, request: Value) -> Result<Value> {
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
        "get" => serde_json::to_value(store.get_with(id()?, &link_options(&request)?)?)?,
        "view" => {
            let kind = field("type")?.as_str().context("type must be a string")?;
            serde_json::to_value(types::view(store, kind, id()?, &link_options(&request)?)?)?
        }
        "links" => serde_json::to_value(store.links(id()?)?)?,
        "stats" => serde_json::to_value(store.stats()?)?,
        "query" => {
            let query: Query = serde_json::from_value(field("query")?.clone())?;
            serde_json::to_value(store.query(&query)?)?
        }
        "create_many" => {
            let nodes: Vec<NewNode> = serde_json::from_value(field("nodes")?.clone())?;
            json!({"ids":store.create_many(nodes)?})
        }
        "batch" => {
            let mutations: Vec<Mutation> = serde_json::from_value(field("ops")?.clone())?;
            let results = store.write(|writer| {
                mutations
                    .into_iter()
                    .map(|op| op.run(writer))
                    .collect::<Result<Vec<_>>>()
            })?;
            json!({"results":results})
        }
        _ => {
            let mutation: Mutation = serde_json::from_value(request)?;
            store.write(|writer| mutation.run(writer))?
        }
    })
}

fn link_options(request: &Value) -> Result<LinkOptions> {
    let mut options = LinkOptions::default();
    if let Some(mode) = request.get("links") {
        options.mode = serde_json::from_value(mode.clone())?;
    }
    if let Some(limit) = request.get("link_limit") {
        options.limit = serde_json::from_value(limit.clone())?;
    }
    Ok(options)
}
