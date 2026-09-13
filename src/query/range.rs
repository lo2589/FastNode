use super::Engine;
use crate::{NodeSet, index};
use anyhow::{Result, bail, ensure};
use rusqlite::{OptionalExtension, params_from_iter, types::Value as SqlValue};
use serde_json::Value;
use std::ops::Bound;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RangeKind {
    Number,
    String,
}

/// gt, gte, lt, lte.
pub(super) type Bounds<'a> = [&'a Option<Value>; 4];

/// All bounds of one range are numbers or all are strings; none means numbers.
pub(super) fn kind(bounds: Bounds) -> Result<RangeKind> {
    let mut kind = None;
    for bound in bounds.into_iter().flatten() {
        let this = match bound {
            Value::Number(n) => {
                index::number_key(n)?;
                RangeKind::Number
            }
            Value::String(_) => RangeKind::String,
            _ => bail!("range bounds must be numbers or strings"),
        };
        ensure!(
            kind.as_ref().is_none_or(|k| *k == this),
            "range bounds must share one type"
        );
        kind = Some(this);
    }
    Ok(kind.unwrap_or(RangeKind::Number))
}

impl Engine<'_> {
    pub(super) fn range(
        &self,
        field: &str,
        bounds: Bounds,
        candidate: Option<&NodeSet>,
    ) -> Result<NodeSet> {
        match kind(bounds)? {
            RangeKind::String => self.string_range(field, bounds, candidate),
            RangeKind::Number => self.number_range(field, bounds, candidate),
        }
    }

    // String tokens are ordered by their UTF-8 bytes inside the s: prefix.
    fn string_range(
        &self,
        field: &str,
        [gt, gte, lt, lte]: Bounds,
        candidate: Option<&NodeSet>,
    ) -> Result<NodeSet> {
        let text = |v: &Option<Value>| v.as_ref().and_then(Value::as_str).map(|s| format!("s:{s}"));
        let lower = match (text(gt), text(gte)) {
            (Some(t), _) => Bound::Excluded(t),
            (_, Some(t)) => Bound::Included(t),
            _ => Bound::Included("s:".into()),
        };
        let upper = match (text(lt), text(lte)) {
            (Some(t), _) => Bound::Excluded(t),
            (_, Some(t)) => Bound::Included(t),
            _ => Bound::Excluded("s;".into()),
        };
        self.token_range(field, lower, upper, candidate)
    }

    fn number_range(
        &self,
        field: &str,
        [gt, gte, lt, lte]: Bounds,
        candidate: Option<&NodeSet>,
    ) -> Result<NodeSet> {
        let mut clause = "field=?1".to_owned();
        let mut args = vec![SqlValue::Text(field.into())];
        for (op, bound) in [(">", gt), (">=", gte), ("<", lt), ("<=", lte)] {
            if let Some(Value::Number(number)) = bound {
                args.push(SqlValue::Blob(index::number_key(number)?));
                clause.push_str(&format!(" AND value{op}?{}", args.len()));
            }
        }
        let mut result = NodeSet::new();
        if let Some(candidate) = candidate.filter(|s| s.len() <= 2048) {
            args.push(SqlValue::Null);
            let mut stmt = self.conn.prepare_cached(&format!(
                "SELECT 1 FROM numbers WHERE {clause} AND node_id=?{} LIMIT 1",
                args.len()
            ))?;
            for id in candidate {
                *args.last_mut().unwrap() = SqlValue::Integer(id as i64);
                let hit = stmt
                    .query_row(params_from_iter(&args), |_| Ok(()))
                    .optional()?;
                if hit.is_some() {
                    result.insert(id);
                }
            }
        } else {
            let mut stmt = self
                .conn
                .prepare_cached(&format!("SELECT node_id FROM numbers WHERE {clause}"))?;
            let mut rows = stmt.query(params_from_iter(&args))?;
            while let Some(row) = rows.next()? {
                result.insert(row.get(0)?);
            }
            if let Some(candidate) = candidate {
                result &= candidate;
            }
        }
        Ok(result)
    }
}
