use super::schema::{Field, Schema, Source, View};
use super::span::Span;
use crate::Direction::{In, Out};
use crate::{LinkOptions, LinkRef, NewNode, NodeId, OrderBy, Predicate, Query, Step, Store};
use anyhow::Result;
use serde::Serialize;
use serde_json::json;

/// event: `time` is a Span (roughly when); `next` / `before` record which
/// happened first, independently of how precise the spans are.
pub static SCHEMA: Schema = Schema {
    kind: "event",
    fields: &[
        Field {
            name: "what",
            source: Source::Attr("/what"),
        },
        Field {
            name: "time",
            source: Source::Attr("/time"),
        },
        Field {
            name: "next",
            source: Source::Link {
                relation: "next",
                direction: Out,
                many: false,
            },
        },
        Field {
            name: "before",
            source: Source::Link {
                relation: "next",
                direction: In,
                many: false,
            },
        },
    ],
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventNode {
    pub id: NodeId,
    pub summary: String,
    pub what: Option<String>,
    pub time: Option<Span>,
    pub next: Option<LinkRef>,
    pub before: Option<LinkRef>,
}

impl EventNode {
    pub fn create(db: &mut Store, what: &str, summary: &str, time: Span) -> Result<NodeId> {
        let attrs = json!({"what": what, "time": time});
        db.create(NewNode::new(SCHEMA.kind, summary, attrs))
    }

    /// Records that `earlier` happened right before `later`.
    pub fn then(db: &mut Store, earlier: NodeId, later: NodeId) -> Result<bool> {
        db.write(|w| SCHEMA.link(w, earlier, "next", later))
    }

    pub fn read(db: &mut Store, id: NodeId) -> Result<Option<Self>> {
        SCHEMA
            .read(db, id, &LinkOptions::default())?
            .map(|view| Self::from_view(&view))
            .transpose()
    }

    /// Events whose time overlaps [start, end), ordered by start then id.
    pub fn during(db: &mut Store, start: i64, end: i64, limit: usize) -> Result<Vec<Self>> {
        let mut query = Query::new(Predicate::And {
            args: vec![SCHEMA.all(), Span::overlap("/time", start, end)],
        });
        query.limit = limit;
        query.order_by = Some(OrderBy::asc("/time/start"));
        let ids = db.query(&query)?.ids;
        Self::read_many(db, &ids)
    }

    /// Everything recorded as happening after `id`, in id order.
    pub fn after(db: &mut Store, id: NodeId) -> Result<Vec<Self>> {
        let mut step = Step::new("next", Out);
        step.min = Some(1);
        let reached = db.select(&Predicate::Traverse {
            from: Box::new(Predicate::Ids { ids: vec![id] }),
            steps: vec![step],
        })?;
        Self::read_many(db, &reached.iter().collect::<Vec<_>>())
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
            what: view.get("what")?,
            time: view.get("time")?,
            next: view.get("next")?,
            before: view.get("before")?,
        })
    }
}
