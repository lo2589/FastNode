use super::schema::{Field, Schema, Source, View};
use crate::Direction::{In, Out};
use crate::{Direction, LinkOptions, LinkRef, NewNode, NodeId, NodeSet, Predicate, Step, Store};
use anyhow::Result;
use serde::Serialize;
use serde_json::json;

/// tree: `parent` is the `parent` link out, `children` the same relation in;
/// `next` / `before` are the sibling order along `next`.
pub static SCHEMA: Schema = Schema {
    kind: "tree",
    fields: &[
        Field {
            name: "name",
            source: Source::Attr("/name"),
        },
        Field {
            name: "parent",
            source: Source::Link {
                relation: "parent",
                direction: Out,
                many: false,
            },
        },
        Field {
            name: "children",
            source: Source::Link {
                relation: "parent",
                direction: In,
                many: true,
            },
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
pub struct TreeNode {
    pub id: NodeId,
    pub summary: String,
    pub name: Option<String>,
    pub parent: Option<LinkRef>,
    pub children: Vec<LinkRef>,
    pub next: Option<LinkRef>,
    pub before: Option<LinkRef>,
}

impl TreeNode {
    /// Creates a tree Node, optionally under `parent`, in one transaction.
    pub fn create(
        db: &mut Store,
        name: &str,
        summary: &str,
        parent: Option<NodeId>,
    ) -> Result<NodeId> {
        db.write(|w| {
            let id = w.create(NewNode::new(SCHEMA.kind, summary, json!({"name": name})))?;
            if let Some(parent) = parent {
                SCHEMA.link(w, id, "parent", parent)?;
            }
            Ok(id)
        })
    }

    pub fn read(db: &mut Store, id: NodeId) -> Result<Option<Self>> {
        SCHEMA
            .read(db, id, &LinkOptions::default())?
            .map(|view| Self::from_view(&view))
            .transpose()
    }

    /// Places `next` directly after `id` in sibling order.
    pub fn set_next(db: &mut Store, id: NodeId, next: NodeId) -> Result<bool> {
        db.write(|w| SCHEMA.link(w, id, "next", next))
    }

    /// Every Node below `id`, at any depth.
    pub fn descendants(db: &mut Store, id: NodeId) -> Result<NodeSet> {
        db.select(&walk(id, In))
    }

    /// Every Node above `id`, up to the root.
    pub fn ancestors(db: &mut Store, id: NodeId) -> Result<NodeSet> {
        db.select(&walk(id, Out))
    }

    fn from_view(view: &View) -> Result<Self> {
        Ok(Self {
            id: view.node.id,
            summary: view.node.summary.clone(),
            name: view.get("name")?,
            parent: view.get("parent")?,
            children: view.get("children")?,
            next: view.get("next")?,
            before: view.get("before")?,
        })
    }
}

fn walk(id: NodeId, direction: Direction) -> Predicate {
    let mut step = Step::new("parent", direction);
    step.min = Some(1);
    Predicate::Traverse {
        from: Box::new(Predicate::Ids { ids: vec![id] }),
        steps: vec![step],
    }
}
