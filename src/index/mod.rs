//! Turns a Node into index entries: typed postings, number keys and intervals.

mod path;
mod value;
mod visit;

pub(crate) use path::{TYPE_FIELD, field_valid, is_wildcard};
pub(crate) use value::{
    INFINITY, hex, number_key, number_token, span_length, token, token_sort_bytes, unhex,
};

use crate::NewNode;
use anyhow::{Result, ensure};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const LIVE: &str = "u:";
pub(crate) const EXISTS: &str = "e:";
/// Posted at a path where one Node holds more than one sortable value.
pub(crate) const MULTI: &str = "m:";
/// Posted at a path where a Node holds at least one sortable value.
pub(crate) const ORDERED: &str = "o:";

/// (field, token) → number key when the token is a number.
pub(crate) type Postings = BTreeMap<(String, String), Option<Vec<u8>>>;
/// (field, start key, end key); an open end is INFINITY.
pub(crate) type Intervals = BTreeSet<(String, Vec<u8>, Vec<u8>)>;

#[derive(Debug, Default)]
pub(crate) struct Attributes {
    pub postings: Postings,
    pub intervals: Intervals,
}

pub(crate) fn validate(node: &NewNode) -> Result<()> {
    ensure!(!node.kind.trim().is_empty(), "type is required");
    ensure!(!node.summary.trim().is_empty(), "summary is required");
    ensure!(node.attrs.is_object(), "attrs must be a JSON object");
    Ok(())
}

pub(crate) fn attributes(kind: &str, attrs: &Value) -> Result<Attributes> {
    let mut out = Attributes::default();
    out.postings.insert((String::new(), LIVE.into()), None);
    out.postings.insert(
        (TYPE_FIELD.into(), token(&Value::String(kind.into()))?.0),
        None,
    );
    visit::visit("", None, attrs, &mut out)?;
    mark_sortable(&mut out.postings);
    Ok(out)
}

pub(crate) fn is_sortable(token: &str) -> bool {
    token.starts_with("n:") || token.starts_with("s:")
}

// Per path: `o:` when this Node has a sortable value, `m:` when it has several.
fn mark_sortable(postings: &mut Postings) {
    let mut per_field: BTreeMap<&str, usize> = BTreeMap::new();
    for (field, token) in postings.keys() {
        if is_sortable(token) {
            *per_field.entry(field).or_default() += 1;
        }
    }
    let marks: Vec<(String, usize)> = per_field
        .into_iter()
        .map(|(field, n)| (field.to_owned(), n))
        .collect();
    for (field, n) in marks {
        if n > 1 {
            postings.insert((field.clone(), MULTI.into()), None);
        }
        postings.insert((field, ORDERED.into()), None);
    }
}

pub(crate) fn merge_patch(target: &mut Value, patch: &Value) {
    if let Value::Object(patch) = patch {
        if !target.is_object() {
            *target = Value::Object(Default::default());
        }
        let target = target.as_object_mut().unwrap();
        for (key, value) in patch {
            if value.is_null() {
                target.remove(key);
            } else {
                merge_patch(target.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
    } else {
        *target = patch.clone();
    }
}
