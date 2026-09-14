//! seek: one group of a declared composite index, in ordering order.

use super::Engine;
use super::range::{Bounds, RangeKind, kind};
use crate::store::composite::{STRING_ORDER, definition, group_key, order_key};
use crate::{NodeSet, SortDirection, index};
use anyhow::{Result, ensure};
use rusqlite::{params_from_iter, types::Value as SqlValue};
use serde_json::Value;

impl Engine<'_> {
    /// Up to `limit` entries of `group`, nearest first in `direction`, within
    /// the bounds; restricted to `candidate` after the limit is applied.
    pub(super) fn seek(
        &self,
        name: &str,
        group: &[Value],
        bounds: Bounds,
        direction: SortDirection,
        limit: Option<usize>,
        candidate: Option<&NodeSet>,
    ) -> Result<NodeSet> {
        let def = definition(self.conn, name)?;
        ensure!(
            group.len() == def.grouping.len(),
            "index {name} groups by {} values, got {}",
            def.grouping.len(),
            group.len()
        );
        let tokens = group
            .iter()
            .map(|value| Ok(index::token(value)?.0))
            .collect::<Result<Vec<_>>>()?;
        let mut sql = String::from("SELECT node_id FROM composites WHERE name=?1 AND grp=?2");
        let mut args = vec![
            SqlValue::Text(name.into()),
            SqlValue::Blob(group_key(&tokens)),
        ];
        let [gt, gte, lt, lte] = bounds;
        for (op, bound) in [(">", gt), (">=", gte), ("<", lt), ("<=", lte)] {
            if let Some(value) = bound {
                args.push(SqlValue::Blob(order_key(&index::token(value)?.0)?));
                sql.push_str(&format!(" AND ord{op}?{}", args.len()));
            }
        }
        // A bound picks a type: number bounds never reach strings and back.
        if bounds.iter().any(|b| b.is_some()) {
            sql.push_str(match kind(bounds)? {
                RangeKind::Number => " AND ord<X'20'",
                RangeKind::String => " AND ord>=X'20'",
            });
        }
        debug_assert_eq!(STRING_ORDER, 0x20);
        sql.push_str(match direction {
            SortDirection::Asc => " ORDER BY ord,node_id",
            SortDirection::Desc => " ORDER BY ord DESC,node_id DESC",
        });
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut rows = stmt.query(params_from_iter(&args))?;
        let mut result = NodeSet::new();
        while let Some(row) = rows.next()? {
            result.insert(row.get(0)?);
        }
        if let Some(candidate) = candidate {
            result &= candidate;
        }
        Ok(result)
    }
}
