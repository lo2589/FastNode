//! Command-line arguments → the same JSON requests the rpc mode accepts.

use anyhow::{Context, Result, bail};
use fastnode::{OrderBy, Predicate, Query, SortDirection};
use serde_json::{Value, json};
use std::{fs::File, io};

pub fn request(command: &str, args: &[String]) -> Result<Value> {
    let arg = |i: usize| {
        args.get(i)
            .map(String::as_str)
            .with_context(|| format!("missing argument {}; see --help", i + 1))
    };
    let id = |i: usize| -> Result<u32> { Ok(arg(i)?.parse()?) };
    Ok(match command {
        "create" => json!({"op":"create","node":input(arg(0)?)?}),
        "create-many" => json!({"op":"create_many","nodes":input(arg(0)?)?}),
        "get" => with_link_flags(json!({"op":"get","id":id(0)?}), args)?,
        "view" => with_link_flags(json!({"op":"view","type":arg(0)?,"id":id(1)?}), args)?,
        "find" => find(args)?,
        "delete" | "links" => json!({"op":command,"id":id(0)?}),
        "replace" => json!({"op":"replace","id":id(0)?,"node":input(arg(1)?)?}),
        "patch" => json!({"op":"patch","id":id(0)?,"patch":input(arg(1)?)?}),
        "link" | "unlink" => {
            json!({"op":command,"from":id(0)?,"relation":arg(1)?,"to":id(2)?})
        }
        "query" => json!({"op":"query","query":input(arg(0)?)?}),
        "batch" => json!({"op":"batch","ops":input(arg(0)?)?}),
        "stats" => json!({"op":"stats"}),
        _ => bail!("unknown command {command}; see --help"),
    })
}

fn with_link_flags(mut request: Value, args: &[String]) -> Result<Value> {
    for (name, value) in flags(args)?.1 {
        match name {
            "links" => request["links"] = json!(value),
            "link-limit" => request["link_limit"] = json!(value.parse::<usize>()?),
            _ => bail!("unknown flag --{name}"),
        }
    }
    Ok(request)
}

fn find(args: &[String]) -> Result<Value> {
    let (positional, named) = flags(args)?;
    let value = scalar(positional.first().context("find requires a value")?);
    let mut query = Query::new(Predicate::Any { value });
    query.include_data = true;
    let mut direction = SortDirection::Asc;
    for (name, value) in named {
        match name {
            "links" => query.links = serde_json::from_value(json!(value))?,
            "link-limit" => query.link_limit = value.parse()?,
            "limit" => query.limit = value.parse()?,
            "after" => query.after = value.parse()?,
            "order" => query.order_by = Some(OrderBy::asc(value)),
            "direction" => direction = serde_json::from_value(json!(value))?,
            "cursor" => query.cursor = Some(value.into()),
            _ => bail!("unknown find flag --{name}"),
        }
    }
    if let Some(order) = &mut query.order_by {
        order.direction = direction;
    }
    Ok(json!({"op":"query","query":query}))
}

pub fn input(arg: &str) -> Result<Value> {
    if arg == "-" {
        return Ok(serde_json::from_reader(io::stdin().lock())?);
    }
    if let Some(path) = arg.strip_prefix('@') {
        return Ok(serde_json::from_reader(File::open(path)?)?);
    }
    Ok(serde_json::from_str(arg)?)
}

// A bare word is a string; anything that parses as a JSON scalar keeps its
// type, so `26` is a number and `'"26"'` is a string.
fn scalar(arg: &str) -> Value {
    serde_json::from_str::<Value>(arg)
        .ok()
        .filter(|v| !v.is_object() && !v.is_array())
        .unwrap_or_else(|| Value::String(arg.into()))
}

type Flags<'a> = (Vec<&'a str>, Vec<(&'a str, &'a str)>);

// Splits `--name value` pairs from positional arguments.
fn flags(args: &[String]) -> Result<Flags<'_>> {
    let mut positional = Vec::new();
    let mut named = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if let Some(name) = arg.strip_prefix("--") {
            let value = iter
                .next()
                .with_context(|| format!("--{name} requires a value"))?;
            named.push((name, value.as_str()));
        } else {
            positional.push(arg.as_str());
        }
    }
    Ok((positional, named))
}
