//! Derived Node types. Each lifts attrs paths and link relations to first-level
//! fields (`node.name`, `node.next`) while storage stays Node + links, and
//! every read, query and write translates to core operations.
//!
//! Beyond the built-in schemas below, `define_type` stores a `TypeDef` in the
//! database and `view` resolves it at read time — no recompile needed.

pub mod code;
pub mod event;
pub mod schema;
pub mod span;
pub mod state;
pub mod tree;

pub use code::{CodeNode, RelationshipNode};
pub use event::EventNode;
pub use schema::{Field, FieldDef, Schema, Source, TypeDef, View};
pub use span::Span;
pub use state::StateNode;
pub use tree::TreeNode;

use crate::{LinkOptions, NodeId, Store};
use anyhow::{Context, Result, ensure};
use rusqlite::OptionalExtension;

/// Every registered built-in schema.
pub fn schemas() -> [&'static Schema; 5] {
    [&tree::SCHEMA, &event::SCHEMA, &state::SCHEMA, &code::CODE, &code::RELATIONSHIP]
}

pub fn schema(kind: &str) -> Option<&'static Schema> {
    schemas().into_iter().find(|s| s.kind == kind)
}

/// Stores a runtime derived-type definition. Declaring the identical
/// definition again does nothing; a conflicting one for the same kind fails.
pub fn define_type(db: &mut Store, def: &TypeDef) -> Result<()> {
    def.validate()?;
    ensure!(
        schema(&def.kind).is_none(),
        "{} is a built-in type and cannot be redefined",
        def.kind
    );
    db.write(|w| {
        let existing: Option<String> = w
            .tx
            .query_row("SELECT def FROM typedefs WHERE kind=?1", [&def.kind], |r| r.get(0))
            .optional()?;
        let encoded = serde_json::to_string(def)?;
        if let Some(existing) = existing {
            ensure!(
                existing == encoded,
                "type {} already exists with a different definition",
                def.kind
            );
            return Ok(());
        }
        w.tx.execute("INSERT INTO typedefs(kind,def) VALUES(?1,?2)", [&def.kind, &encoded])?;
        Ok(())
    })
}

/// Every runtime-defined derived type, ordered by kind.
pub fn type_defs(db: &mut Store) -> Result<Vec<TypeDef>> {
    let rows = db
        .conn
        .prepare("SELECT def FROM typedefs ORDER BY kind")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.iter()
        .map(|row| serde_json::from_str(row).with_context(|| format!("corrupt typedef: {row}")))
        .collect()
}

fn load_type_def(db: &Store, kind: &str) -> Result<Option<TypeDef>> {
    let row: Option<String> = db
        .conn
        .query_row("SELECT def FROM typedefs WHERE kind=?1", [kind], |r| r.get(0))
        .optional()?;
    row.map(|s| serde_json::from_str(&s).with_context(|| format!("corrupt typedef for {kind}")))
        .transpose()
}

/// Reads Node `id` through the schema named `kind`: built-in first, then
/// runtime-defined types stored in the database.
pub fn view(db: &mut Store, kind: &str, id: NodeId, options: &LinkOptions) -> Result<Option<View>> {
    if let Some(schema) = schema(kind) {
        return schema.read(db, id, options);
    }
    load_type_def(db, kind)?
        .with_context(|| format!("unknown type {kind}"))?
        .read(db, id, options)
}
