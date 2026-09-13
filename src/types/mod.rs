//! Derived Node types. Each lifts attrs paths and link relations to first-level
//! fields (`node.name`, `node.next`) while storage stays Node + links, and
//! every read, query and write translates to core operations.

pub mod event;
pub mod schema;
pub mod span;
pub mod state;
pub mod tree;

pub use event::EventNode;
pub use schema::{Field, Schema, Source, View};
pub use span::Span;
pub use state::StateNode;
pub use tree::TreeNode;

use crate::{LinkOptions, NodeId, Store};
use anyhow::{Context, Result};

/// Every registered schema.
pub fn schemas() -> [&'static Schema; 3] {
    [&tree::SCHEMA, &event::SCHEMA, &state::SCHEMA]
}

pub fn schema(kind: &str) -> Option<&'static Schema> {
    schemas().into_iter().find(|s| s.kind == kind)
}

/// Reads Node `id` through the schema named `kind`.
pub fn view(db: &mut Store, kind: &str, id: NodeId, options: &LinkOptions) -> Result<Option<View>> {
    schema(kind)
        .with_context(|| format!("unknown type {kind}"))?
        .read(db, id, options)
}
