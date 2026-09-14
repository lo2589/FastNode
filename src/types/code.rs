use super::schema::{Field, Schema, Source, View};
use crate::Direction::{In, Out};
use crate::{LinkOptions, LinkRef, NewNode, NodeId, Store};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

/// code: a code symbol (function/method/class/module). Structural relations
/// (`call`, `dataflow`, `impact`) lift to first-level fields in both
/// directions; symbol metadata stays in attrs.
pub static CODE: Schema = Schema {
    kind: "code",
    fields: &[
        Field { name: "symbol", source: Source::Attr("/symbol") },
        Field { name: "qualified_name", source: Source::Attr("/qualified_name") },
        Field { name: "symbol_type", source: Source::Attr("/symbol_type") },
        Field { name: "file", source: Source::Attr("/file") },
        Field { name: "line_start", source: Source::Attr("/line_start") },
        Field { name: "line_end", source: Source::Attr("/line_end") },
        Field { name: "hash", source: Source::Attr("/hash") },
        Field { name: "imports", source: Source::Attr("/imports") },
        Field { name: "callers", source: Source::Attr("/callers") },
        Field { name: "inputs", source: Source::Attr("/inputs") },
        Field { name: "calls", source: Source::Link { relation: "call", direction: Out, many: true } },
        Field { name: "called_by", source: Source::Link { relation: "call", direction: In, many: true } },
        Field { name: "flows_to", source: Source::Link { relation: "dataflow", direction: Out, many: true } },
        Field { name: "flows_from", source: Source::Link { relation: "dataflow", direction: In, many: true } },
        Field { name: "impacts", source: Source::Link { relation: "impact", direction: Out, many: true } },
        Field { name: "impacted_by", source: Source::Link { relation: "impact", direction: In, many: true } },
    ],
};

/// relationship: a reified edge. The endpoints are the `rel_from` / `rel_to`
/// links; every edge attribute (condition, call site, evidence, ...) stays in
/// attrs and is indexed like any other field.
pub static RELATIONSHIP: Schema = Schema {
    kind: "relationship",
    fields: &[
        Field { name: "from", source: Source::Link { relation: "rel_from", direction: Out, many: false } },
        Field { name: "to", source: Source::Link { relation: "rel_to", direction: Out, many: false } },
        Field { name: "edge_type", source: Source::Attr("/edge_type") },
        Field { name: "condition", source: Source::Attr("/condition") },
        Field { name: "data_in", source: Source::Attr("/data_in") },
        Field { name: "data_out", source: Source::Attr("/data_out") },
        Field { name: "outer_hash", source: Source::Attr("/outer_hash") },
        Field { name: "dispatch_key", source: Source::Attr("/dispatch_key") },
        Field { name: "dispatch_value", source: Source::Attr("/dispatch_value") },
        Field { name: "is_conditional", source: Source::Attr("/is_conditional") },
        Field { name: "scope_label", source: Source::Attr("/scope_label") },
        Field { name: "seq", source: Source::Attr("/seq") },
        Field { name: "call_line", source: Source::Attr("/call_line") },
        Field { name: "call_column", source: Source::Attr("/call_column") },
        Field { name: "resolution", source: Source::Attr("/resolution") },
        Field { name: "evidence", source: Source::Attr("/evidence") },
        Field { name: "bridge_kind", source: Source::Attr("/bridge_kind") },
    ],
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CodeNode {
    pub id: NodeId,
    pub summary: String,
    pub symbol: Option<String>,
    pub file: Option<String>,
    pub line_start: Option<i64>,
    pub calls: Vec<LinkRef>,
    pub called_by: Vec<LinkRef>,
}

impl CodeNode {
    pub fn create(db: &mut Store, summary: &str, attrs: Value) -> Result<NodeId> {
        db.create(NewNode::new(CODE.kind, summary, attrs))
    }

    pub fn read(db: &mut Store, id: NodeId) -> Result<Option<Self>> {
        CODE.read(db, id, &LinkOptions::default())?
            .map(|view| Self::from_view(&view))
            .transpose()
    }

    fn from_view(view: &View) -> Result<Self> {
        Ok(Self {
            id: view.node.id,
            summary: view.node.summary.clone(),
            symbol: view.get("symbol")?,
            file: view.get("file")?,
            line_start: view.get("line_start")?,
            calls: view.get("calls")?,
            called_by: view.get("called_by")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RelationshipNode {
    pub id: NodeId,
    pub summary: String,
    pub from: Option<LinkRef>,
    pub to: Option<LinkRef>,
    pub edge_type: Option<String>,
    pub call_line: Option<i64>,
    pub is_conditional: Option<bool>,
}

impl RelationshipNode {
    /// Creates a relationship Node between `from` and `to` in one transaction.
    pub fn create(
        db: &mut Store,
        summary: &str,
        attrs: Value,
        from: NodeId,
        to: NodeId,
    ) -> Result<NodeId> {
        db.write(|w| {
            let id = w.create(NewNode::new(RELATIONSHIP.kind, summary, attrs))?;
            RELATIONSHIP.link(w, id, "from", from)?;
            RELATIONSHIP.link(w, id, "to", to)?;
            Ok(id)
        })
    }

    pub fn read(db: &mut Store, id: NodeId) -> Result<Option<Self>> {
        RELATIONSHIP
            .read(db, id, &LinkOptions::default())?
            .map(|view| Self::from_view(&view))
            .transpose()
    }

    fn from_view(view: &View) -> Result<Self> {
        Ok(Self {
            id: view.node.id,
            summary: view.node.summary.clone(),
            from: view.get("from")?,
            to: view.get("to")?,
            edge_type: view.get("edge_type")?,
            call_line: view.get("call_line")?,
            is_conditional: view.get("is_conditional")?,
        })
    }
}
