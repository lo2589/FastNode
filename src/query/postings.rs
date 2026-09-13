//! Reads of the bitmap postings and their counts.

use super::Engine;
use crate::NodeSet;
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeSet;
use std::ops::Bound;

impl Engine<'_> {
    pub(super) fn count(&self, field: &str, token: &str) -> Result<u64> {
        Ok(self
            .conn
            .prepare_cached("SELECT n FROM counts WHERE field=?1 AND token=?2")?
            .query_row(params![field, token], |r| r.get::<_, i64>(0))
            .optional()?
            .unwrap_or(0) as u64)
    }

    /// Whether `field` has any posting token in [lo, hi).
    pub(super) fn has_tokens(&self, field: &str, lo: &str, hi: &str) -> Result<bool> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT 1 FROM counts WHERE field=?1 AND token>=?2 AND token<?3 LIMIT 1",
            )?
            .query_row(params![field, lo, hi], |_| Ok(()))
            .optional()?
            .is_some())
    }

    pub(super) fn posting(
        &self,
        field: &str,
        token: &str,
        candidate: Option<&NodeSet>,
    ) -> Result<NodeSet> {
        let mut result = NodeSet::new();
        if let Some(candidate) = candidate.filter(|s| s.len() < 32768) {
            let shards: BTreeSet<u32> = candidate.iter().map(|id| id >> 16).collect();
            let mut stmt = self.conn.prepare_cached(
                "SELECT bitmap FROM postings WHERE field=?1 AND token=?2 AND shard=?3",
            )?;
            for shard in shards {
                let blob: Option<Vec<u8>> = stmt
                    .query_row(params![field, token, shard], |r| r.get(0))
                    .optional()?;
                if let Some(blob) = blob {
                    result |= NodeSet::deserialize_from(&blob[..])? & candidate;
                }
            }
        } else {
            let mut stmt = self.conn.prepare_cached(
                "SELECT bitmap FROM postings WHERE field=?1 AND token=?2 ORDER BY shard",
            )?;
            let mut rows = stmt.query(params![field, token])?;
            while let Some(row) = rows.next()? {
                let blob: Vec<u8> = row.get(0)?;
                result |= NodeSet::deserialize_from(&blob[..])?;
            }
            if let Some(candidate) = candidate {
                result &= candidate;
            }
        }
        Ok(result)
    }

    /// Union of every posting of `field` whose token lies between the bounds.
    pub(super) fn token_range(
        &self,
        field: &str,
        lower: Bound<String>,
        upper: Bound<String>,
        candidate: Option<&NodeSet>,
    ) -> Result<NodeSet> {
        let (lo_op, lo) = match lower {
            Bound::Included(t) => (">=", t),
            Bound::Excluded(t) => (">", t),
            Bound::Unbounded => bail!("token ranges need a lower bound"),
        };
        let (hi_op, hi) = match upper {
            Bound::Included(t) => ("<=", t),
            Bound::Excluded(t) => ("<", t),
            Bound::Unbounded => bail!("token ranges need an upper bound"),
        };
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT bitmap FROM postings WHERE field=?1 AND token{lo_op}?2 AND token{hi_op}?3"
        ))?;
        let mut rows = stmt.query(params![field, lo, hi])?;
        let mut set = NodeSet::new();
        while let Some(row) = rows.next()? {
            let blob: Vec<u8> = row.get(0)?;
            let mut block = NodeSet::deserialize_from(&blob[..])?;
            if let Some(candidate) = candidate {
                block &= candidate;
            }
            set |= block;
        }
        Ok(set)
    }
}

/// The smallest string greater than every string starting with `prefix`.
pub(super) fn prefix_end(prefix: &str) -> String {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        let mut next = last as u32 + 1;
        if next == 0xD800 {
            next = 0xE000;
        }
        if let Some(next) = char::from_u32(next) {
            chars.push(next);
            return chars.into_iter().collect();
        }
    }
    unreachable!("typed string prefixes always start with ASCII s:")
}
