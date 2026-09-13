//! Interval relations. A row is [lo, hi): lo == hi is an instant, and hi is
//! INFINITY for an open interval. Each row keeps its own start and end, so
//! intervals inside arrays never pair a start with another element's end.
//!
//! A full scan only reads starts that can still match: a finite interval no
//! longer than L that reaches past `t` starts after `t - L`, where L is the
//! path's recorded maximum length. Open intervals come from their own index.

use super::Engine;
use crate::{NodeSet, index};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::Number;

#[derive(Debug, Clone, Copy)]
pub(super) enum IntervalOp {
    At,
    Overlap,
    Contains,
    ContainedBy,
}

impl IntervalOp {
    // ?2 and ?3 are the query start and end; At passes the instant twice.
    fn condition(self) -> &'static str {
        match self {
            Self::At => "lo<=?2 AND (hi>?3 OR (hi=lo AND lo=?2))",
            Self::Overlap => "lo<?3 AND (hi>?2 OR (hi=lo AND lo>=?2))",
            Self::Contains => "lo<=?2 AND hi>=?3",
            Self::ContainedBy => "lo>=?2 AND lo<=?3 AND hi<=?3 AND (hi>lo OR lo<?3)",
        }
    }
}

impl Engine<'_> {
    pub(super) fn interval(
        &self,
        field: &str,
        op: IntervalOp,
        start: &Number,
        end: &Number,
        candidate: Option<&NodeSet>,
    ) -> Result<NodeSet> {
        let keys = (index::number_key(start)?, index::number_key(end)?);
        let condition = op.condition();
        let mut result = NodeSet::new();
        if let Some(candidate) = candidate.filter(|s| s.len() <= 2048) {
            let mut stmt = self.conn.prepare_cached(&format!(
                "SELECT 1 FROM intervals WHERE node_id=?4 AND field=?1 AND {condition} LIMIT 1"
            ))?;
            for id in candidate {
                let hit = stmt
                    .query_row(params![field, keys.0, keys.1, id], |_| Ok(()))
                    .optional()?;
                if hit.is_some() {
                    result.insert(id);
                }
            }
            return Ok(result);
        }
        // Contained intervals already start inside [start, end].
        let lowest = match op {
            IntervalOp::ContainedBy => None,
            IntervalOp::Contains => self.lowest_start(field, end)?,
            IntervalOp::At | IntervalOp::Overlap => self.lowest_start(field, start)?,
        };
        let mut stmt;
        let mut rows = match &lowest {
            Some(lowest) => {
                stmt = self.conn.prepare_cached(&format!(
                    "SELECT node_id FROM intervals WHERE field=?1 AND lo>=?4 AND {condition} UNION ALL SELECT node_id FROM intervals WHERE field=?1 AND hi=X'03' AND {condition}"
                ))?;
                stmt.query(params![field, keys.0, keys.1, lowest])?
            }
            None => {
                stmt = self.conn.prepare_cached(&format!(
                    "SELECT node_id FROM intervals WHERE field=?1 AND {condition}"
                ))?;
                stmt.query(params![field, keys.0, keys.1])?
            }
        };
        while let Some(row) = rows.next()? {
            result.insert(row.get(0)?);
        }
        if let Some(candidate) = candidate {
            result &= candidate;
        }
        Ok(result)
    }

    /// The smallest start a finite interval ending after `reach` can have, or
    /// None when the path's lengths are unbounded or `reach` exceeds f64.
    fn lowest_start(&self, field: &str, reach: &Number) -> Result<Option<Vec<u8>>> {
        let bound: Option<Option<f64>> = self
            .conn
            .prepare_cached("SELECT max_len FROM interval_fields WHERE field=?1")?
            .query_row([field], |r| r.get(0))
            .optional()?;
        let longest = match bound {
            None => 0.0,
            Some(None) => return Ok(None),
            Some(Some(longest)) => longest,
        };
        let Some(reach) = reach.as_f64().filter(|r| r.is_finite()) else {
            return Ok(None);
        };
        let lowest = reach - longest - (reach.abs() + longest) * 1e-12 - f64::MIN_POSITIVE;
        Number::from_f64(lowest)
            .map(|n| index::number_key(&n))
            .transpose()
    }
}
