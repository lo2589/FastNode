//! Number rows and interval rows.
//!
//! A number held at a path by a single Node lives only in `numbers`. When a
//! second Node takes the same value it gains a bitmap posting and a count;
//! when it drops back to one holder the posting goes away again. Unique values
//! such as timestamps therefore cost one row instead of three.

use super::Write;
use crate::NodeId;
use crate::index::{self, INFINITY};
use anyhow::Result;
use rusqlite::params;

impl Write<'_> {
    pub(super) fn add_number(
        &mut self,
        key: &(String, String),
        value: &[u8],
        id: NodeId,
    ) -> Result<()> {
        self.tx
            .prepare_cached("INSERT INTO numbers(field,value,node_id) VALUES(?1,?2,?3)")?
            .execute(params![key.0, value, id])?;
        match self.holders(&key.0, value)? {
            1 => Ok(()),
            2 => {
                let other = self.other_holder(&key.0, value, id)?;
                self.membership(key, other, true)?;
                self.membership(key, id, true)
            }
            _ => self.membership(key, id, true),
        }
    }

    pub(super) fn remove_number(
        &mut self,
        key: &(String, String),
        value: &[u8],
        id: NodeId,
    ) -> Result<()> {
        self.tx
            .prepare_cached("DELETE FROM numbers WHERE field=?1 AND value=?2 AND node_id=?3")?
            .execute(params![key.0, value, id])?;
        match self.holders(&key.0, value)? {
            0 => Ok(()),
            1 => {
                let other = self.other_holder(&key.0, value, id)?;
                self.membership(key, other, false)?;
                self.membership(key, id, false)
            }
            _ => self.membership(key, id, false),
        }
    }

    // Holders of one value, counted up to 3.
    fn holders(&self, field: &str, value: &[u8]) -> Result<u32> {
        Ok(self
            .tx
            .prepare_cached(
                "SELECT count(*) FROM (SELECT 1 FROM numbers WHERE field=?1 AND value=?2 LIMIT 3)",
            )?
            .query_row(params![field, value], |r| r.get(0))?)
    }

    fn other_holder(&self, field: &str, value: &[u8], id: NodeId) -> Result<NodeId> {
        Ok(self
            .tx
            .prepare_cached(
                "SELECT node_id FROM numbers WHERE field=?1 AND value=?2 AND node_id<>?3 LIMIT 1",
            )?
            .query_row(params![field, value, id], |r| r.get(0))?)
    }

    pub(super) fn add_interval(
        &mut self,
        field: &str,
        lo: &[u8],
        hi: &[u8],
        id: NodeId,
    ) -> Result<()> {
        self.tx
            .prepare_cached("INSERT INTO intervals(field,lo,hi,node_id) VALUES(?1,?2,?3,?4)")?
            .execute(params![field, lo, hi, id])?;
        if hi != INFINITY {
            self.widen(field, index::span_length(lo, hi))?;
        }
        Ok(())
    }

    pub(super) fn remove_interval(
        &mut self,
        field: &str,
        lo: &[u8],
        hi: &[u8],
        id: NodeId,
    ) -> Result<()> {
        self.tx
            .prepare_cached(
                "DELETE FROM intervals WHERE field=?1 AND lo=?2 AND hi=?3 AND node_id=?4",
            )?
            .execute(params![field, lo, hi, id])?;
        Ok(())
    }

    // interval_fields keeps an upper bound on finite interval length per path;
    // NULL means unbounded. It never shrinks, so it always stays a valid bound.
    fn widen(&mut self, field: &str, length: Option<f64>) -> Result<()> {
        let covered = match (self.spans.get(field), length) {
            (Some(None), _) => true,
            (Some(Some(max)), Some(length)) => length <= *max,
            _ => false,
        };
        if covered {
            return Ok(());
        }
        let max: Option<f64> = self
            .tx
            .prepare_cached("INSERT INTO interval_fields(field,max_len) VALUES(?1,?2) ON CONFLICT(field) DO UPDATE SET max_len=CASE WHEN max_len IS NULL OR excluded.max_len IS NULL THEN NULL ELSE max(max_len,excluded.max_len) END RETURNING max_len")?
            .query_row(params![field, length], |r| r.get(0))?;
        self.spans.insert(field.to_owned(), max);
        Ok(())
    }
}
