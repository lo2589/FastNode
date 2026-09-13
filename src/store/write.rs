use super::get_node;
use super::maintain::{BlockKey, Pending};
use crate::index::{self, Attributes};
use crate::{Link, NewNode, Node, NodeId};
use anyhow::{Context, Result, ensure};
use rusqlite::{Transaction, params};
use serde_json::{Value, json};
use std::collections::HashMap;

/// A write transaction. Index changes are buffered per bitmap block and
/// written once at commit, or earlier when `select` needs them.
pub struct Write<'a> {
    pub(crate) tx: Transaction<'a>,
    pub(super) blocks: HashMap<BlockKey, Pending>,
    /// Interval length bounds already recorded in this transaction.
    pub(super) spans: HashMap<String, Option<f64>>,
}

impl<'a> Write<'a> {
    pub(super) fn new(tx: Transaction<'a>) -> Self {
        Self {
            tx,
            blocks: HashMap::new(),
            spans: HashMap::new(),
        }
    }
    pub(super) fn commit(mut self) -> Result<()> {
        self.flush()?;
        self.tx.commit()?;
        Ok(())
    }
}

impl Write<'_> {
    pub fn get(&self, id: NodeId) -> Result<Option<Node>> {
        get_node(&self.tx, id)
    }

    pub fn create(&mut self, node: NewNode) -> Result<NodeId> {
        index::validate(&node)?;
        let attrs = index::attributes(&node.kind, &node.attrs)?;
        self.tx
            .prepare_cached("INSERT INTO nodes(type,summary,attrs) VALUES(?1,?2,?3)")?
            .execute(params![
                node.kind,
                node.summary,
                serde_json::to_string(&node.attrs)?
            ])?;
        let id = NodeId::try_from(self.tx.last_insert_rowid())?;
        self.change(id, &Attributes::default(), &attrs)?;
        Ok(id)
    }

    pub fn replace(&mut self, id: NodeId, node: NewNode) -> Result<()> {
        index::validate(&node)?;
        let new = index::attributes(&node.kind, &node.attrs)?;
        let old = self
            .get(id)?
            .with_context(|| format!("node {id} does not exist"))?;
        self.change(id, &index::attributes(&old.kind, &old.attrs)?, &new)?;
        self.tx
            .prepare_cached(
                "UPDATE nodes SET type=?1,summary=?2,attrs=?3,version=version+1 WHERE id=?4",
            )?
            .execute(params![
                node.kind,
                node.summary,
                serde_json::to_string(&node.attrs)?,
                id
            ])?;
        Ok(())
    }

    /// JSON Merge Patch over {type, summary, attrs}.
    pub fn patch(&mut self, id: NodeId, patch: Value) -> Result<()> {
        ensure!(
            patch.is_object(),
            "patch must be an object containing type, summary or attrs"
        );
        let node = self
            .get(id)?
            .with_context(|| format!("node {id} does not exist"))?;
        let mut document = json!({"type":node.kind,"summary":node.summary,"attrs":node.attrs});
        index::merge_patch(&mut document, &patch);
        let node: NewNode = serde_json::from_value(document).context("patched node is invalid")?;
        self.replace(id, node)
    }

    pub fn delete(&mut self, id: NodeId) -> Result<bool> {
        let Some(node) = self.get(id)? else {
            return Ok(false);
        };
        let old = index::attributes(&node.kind, &node.attrs)?;
        self.change(id, &old, &Attributes::default())?;
        self.tx
            .prepare_cached("DELETE FROM nodes WHERE id=?1")?
            .execute([id])?;
        Ok(true)
    }

    pub fn link(&mut self, link: &Link) -> Result<bool> {
        ensure!(!link.relation.is_empty(), "relation must not be empty");
        Ok(self
            .tx
            .prepare_cached(
                "INSERT INTO links(source,relation,target) VALUES(?1,?2,?3) ON CONFLICT DO NOTHING",
            )?
            .execute(params![link.from, link.relation, link.to])?
            != 0)
    }

    pub fn unlink(&mut self, link: &Link) -> Result<bool> {
        Ok(self
            .tx
            .prepare_cached("DELETE FROM links WHERE source=?1 AND relation=?2 AND target=?3")?
            .execute(params![link.from, link.relation, link.to])?
            != 0)
    }
}
