use super::schema::{Field, Schema, Source, View};
use super::span::Span;
use crate::Direction::{In, Out};
use crate::{LinkOptions, LinkRef, NewNode, NodeId, OrderBy, Predicate, Query, Step, Store, Write};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Value, json};

/// state: the `value` of one `key` of a subject during `valid`. A subject's
/// history is its states along `state_of`, never overlapping for one key.
pub static SCHEMA: Schema = Schema {
    kind: "state",
    fields: &[
        Field {
            name: "key",
            source: Source::Attr("/key"),
        },
        Field {
            name: "value",
            source: Source::Attr("/value"),
        },
        Field {
            name: "valid",
            source: Source::Attr("/valid"),
        },
        Field {
            name: "subject",
            source: Source::Link {
                relation: "state_of",
                direction: Out,
                many: false,
            },
        },
    ],
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StateNode {
    pub id: NodeId,
    pub summary: String,
    pub key: Option<String>,
    pub value: Value,
    pub valid: Option<Span>,
    pub subject: Option<LinkRef>,
}

impl StateNode {
    /// From `from` on, `key` of `subject` is `value`. A state holding `from`
    /// is cut there and the new state inherits its end; a state starting
    /// exactly at `from` is updated in place.
    pub fn set(
        db: &mut Store,
        subject: NodeId,
        key: &str,
        value: Value,
        from: i64,
        summary: &str,
    ) -> Result<NodeId> {
        db.write(|w| {
            let mut end = None;
            for id in &w.select(&history(subject, key, Some(Span::at("/valid", from))))? {
                let node = w.get(id)?.context("state vanished")?;
                let valid: Span = serde_json::from_value(node.attrs["valid"].clone())?;
                if valid.start == from {
                    w.patch(id, json!({"summary": summary, "attrs": {"value": value}}))?;
                    return Ok(id);
                }
                end = valid.end;
                w.patch(id, json!({"attrs": {"valid": {"end": from}}}))?;
            }
            create(w, subject, key, value, Span::new(from, end), summary)
        })
    }

    /// The state of `key` for `subject` at instant `t`.
    pub fn at(db: &mut Store, subject: NodeId, key: &str, t: i64) -> Result<Option<Self>> {
        let ids = db.select(&history(subject, key, Some(Span::at("/valid", t))))?;
        Ok(Self::read_many(db, &ids.iter().take(1).collect::<Vec<_>>())?.pop())
    }

    /// Every state of `key` for `subject`, oldest first.
    pub fn history(db: &mut Store, subject: NodeId, key: &str) -> Result<Vec<Self>> {
        let mut query = Query::new(history(subject, key, None));
        query.limit = 100_000;
        query.order_by = Some(OrderBy::asc("/valid/start"));
        let ids = db.query(&query)?.ids;
        Self::read_many(db, &ids)
    }

    fn read_many(db: &mut Store, ids: &[NodeId]) -> Result<Vec<Self>> {
        SCHEMA
            .read_many(db, ids, &LinkOptions::default())?
            .iter()
            .flatten()
            .map(Self::from_view)
            .collect()
    }

    fn from_view(view: &View) -> Result<Self> {
        Ok(Self {
            id: view.node.id,
            summary: view.node.summary.clone(),
            key: view.get("key")?,
            value: view.get("value")?,
            valid: view.get("valid")?,
            subject: view.get("subject")?,
        })
    }
}

fn create(
    w: &mut Write,
    subject: NodeId,
    key: &str,
    value: Value,
    valid: Span,
    summary: &str,
) -> Result<NodeId> {
    let attrs = json!({"key": key, "value": value, "valid": valid});
    let id = w.create(NewNode::new(SCHEMA.kind, summary, attrs))?;
    SCHEMA.link(w, id, "subject", subject)?;
    Ok(id)
}

// States of `key` pointing at `subject`, optionally narrowed further.
fn history(subject: NodeId, key: &str, narrow: Option<Predicate>) -> Predicate {
    let of_subject = Predicate::Traverse {
        from: Box::new(Predicate::Ids { ids: vec![subject] }),
        steps: vec![Step::new("state_of", In)],
    };
    let key = Predicate::Eq {
        field: "/key".into(),
        value: json!(key),
    };
    let mut args = vec![SCHEMA.all(), key, of_subject];
    args.extend(narrow);
    Predicate::And { args }
}
