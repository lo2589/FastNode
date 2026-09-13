use super::path::escape;
use super::value::{INFINITY, number_key, token};
use super::{Attributes, EXISTS, Postings};
use anyhow::Result;
use serde_json::{Map, Value};

fn post(out: &mut Postings, path: &str, wild: Option<&str>, key: String, number: Option<Vec<u8>>) {
    if let Some(wild) = wild {
        out.insert((wild.to_owned(), key.clone()), number.clone());
    }
    out.insert((path.to_owned(), key), number);
}

// Each value is posted under its concrete path (/jobs/1/company) and, once an
// array holding objects or arrays has been crossed, under the same path with
// every such index replaced by `*` (/jobs/*/company). Scalar array members are
// already found through the array path itself (/skills), so they get no `*`.
pub(super) fn visit(
    path: &str,
    wild: Option<&str>,
    value: &Value,
    out: &mut Attributes,
) -> Result<()> {
    post(&mut out.postings, path, wild, EXISTS.into(), None);
    match value {
        Value::Object(map) => {
            if let Some((start, end)) = interval(map)? {
                if let Some(wild) = wild {
                    out.intervals
                        .insert((wild.to_owned(), start.clone(), end.clone()));
                }
                out.intervals.insert((path.to_owned(), start, end));
            }
            for (key, value) in map {
                let segment = escape(key);
                let child_wild = wild.map(|w| format!("{w}/{segment}"));
                visit(
                    &format!("{path}/{segment}"),
                    child_wild.as_deref(),
                    value,
                    out,
                )?;
            }
        }
        Value::Array(values) => {
            for (i, value) in values.iter().enumerate() {
                if value.is_object() || value.is_array() {
                    let child_wild = format!("{}/*", wild.unwrap_or(path));
                    visit(&format!("{path}/{i}"), Some(&child_wild), value, out)?;
                } else {
                    visit(&format!("{path}/{i}"), None, value, out)?;
                    let (key, number) = token(value)?;
                    post(&mut out.postings, path, wild, key, number);
                }
            }
        }
        _ => {
            let (key, number) = token(value)?;
            post(&mut out.postings, path, wild, key, number);
        }
    }
    Ok(())
}

// An object with a numeric `start` is the interval [start, end). A missing or
// null end is open; a non-numeric end or an end before start is not indexed.
fn interval(map: &Map<String, Value>) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    let Some(Value::Number(start)) = map.get("start") else {
        return Ok(None);
    };
    let start = number_key(start)?;
    let end = match map.get("end") {
        None | Some(Value::Null) => INFINITY.to_vec(),
        Some(Value::Number(end)) => number_key(end)?,
        Some(_) => return Ok(None),
    };
    Ok((end >= start).then_some((start, end)))
}
