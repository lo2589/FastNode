//! Exact matches. A number held by a single Node at a path has no bitmap
//! posting (see store/numbers.rs), so it is read from the number rows.

use super::Engine;
use crate::NodeSet;
use crate::index::{self, EXISTS};
use anyhow::Result;
use rusqlite::params;
use serde_json::Value;

impl Engine<'_> {
    /// Nodes whose `field` holds the scalar `value`.
    pub(super) fn exact(
        &self,
        field: &str,
        value: &Value,
        candidate: Option<&NodeSet>,
    ) -> Result<NodeSet> {
        let (token, number) = index::token(value)?;
        match number {
            Some(key) if self.count(field, &token)? == 0 => self.sparse(field, &key, candidate),
            _ => self.posting(field, &token, candidate),
        }
    }

    /// Nodes holding `value` at any concrete attrs path.
    pub(super) fn any(&self, value: &Value, candidate: Option<&NodeSet>) -> Result<NodeSet> {
        let mut result = NodeSet::new();
        for (field, _) in self.any_fields(value)? {
            result |= self.exact(&field, value, candidate)?;
        }
        Ok(result)
    }

    /// Concrete attrs paths holding `value`, each with its holder count.
    pub(super) fn any_fields(&self, value: &Value) -> Result<Vec<(String, u64)>> {
        let (token, number) = index::token(value)?;
        let Some(key) = number else {
            return self.fields_with(&token);
        };
        // A single-holder number has no count row, so every path is checked.
        let mut fields = Vec::new();
        for (field, _) in self.fields_with(EXISTS)? {
            let n = match self.count(&field, &token)? {
                0 => self.sparse_count(&field, &key)?,
                n => n,
            };
            if n > 0 {
                fields.push((field, n));
            }
        }
        Ok(fields)
    }

    // Wildcard paths repeat the same Nodes and @type is not an attribute.
    fn fields_with(&self, token: &str) -> Result<Vec<(String, u64)>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT field,n FROM counts WHERE token=?1")?;
        let rows = stmt.query_map([token], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
        })?;
        let mut fields = Vec::new();
        for row in rows {
            let (field, n) = row?;
            if field.starts_with('/') && !index::is_wildcard(&field) {
                fields.push((field, n));
            }
        }
        Ok(fields)
    }

    fn sparse(&self, field: &str, key: &[u8], candidate: Option<&NodeSet>) -> Result<NodeSet> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT node_id FROM numbers WHERE field=?1 AND value=?2")?;
        let mut rows = stmt.query(params![field, key])?;
        let mut set = NodeSet::new();
        while let Some(row) = rows.next()? {
            set.insert(row.get(0)?);
        }
        if let Some(candidate) = candidate {
            set &= candidate;
        }
        Ok(set)
    }

    fn sparse_count(&self, field: &str, key: &[u8]) -> Result<u64> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT count(*) FROM (SELECT 1 FROM numbers WHERE field=?1 AND value=?2 LIMIT 2)",
            )?
            .query_row(params![field, key], |r| r.get::<_, i64>(0))? as u64)
    }
}
