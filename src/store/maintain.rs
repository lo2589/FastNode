//! Keeps postings, counts, number rows and interval rows in step with Nodes.

use super::Write;
use crate::index::Attributes;
use crate::{NodeId, NodeSet};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use std::collections::HashMap;

pub(crate) type BlockKey = (String, String, u32);

pub(crate) struct Pending {
    original: u64,
    set: NodeSet,
}

impl Write<'_> {
    pub(super) fn change(&mut self, id: NodeId, old: &Attributes, new: &Attributes) -> Result<()> {
        for (key, number) in &old.postings {
            if !new.postings.contains_key(key) {
                match number {
                    Some(value) => self.remove_number(key, value, id)?,
                    None => self.membership(key, id, false)?,
                }
            }
        }
        for (key, number) in &new.postings {
            if !old.postings.contains_key(key) {
                match number {
                    Some(value) => self.add_number(key, value, id)?,
                    None => self.membership(key, id, true)?,
                }
            }
        }
        for (field, lo, hi) in old.intervals.difference(&new.intervals) {
            self.remove_interval(field, lo, hi, id)?;
        }
        for (field, lo, hi) in new.intervals.difference(&old.intervals) {
            self.add_interval(field, lo, hi, id)?;
        }
        Ok(())
    }

    pub(super) fn membership(
        &mut self,
        key: &(String, String),
        id: NodeId,
        insert: bool,
    ) -> Result<()> {
        let block_key = (key.0.clone(), key.1.clone(), id >> 16);
        if !self.blocks.contains_key(&block_key) {
            let blob: Option<Vec<u8>> = self
                .tx
                .prepare_cached(
                    "SELECT bitmap FROM postings WHERE field=?1 AND token=?2 AND shard=?3",
                )?
                .query_row(params![key.0, key.1, id >> 16], |r| r.get(0))
                .optional()?;
            let set = match blob {
                Some(blob) => NodeSet::deserialize_from(&blob[..])?,
                None => NodeSet::new(),
            };
            let original = set.len();
            self.blocks
                .insert(block_key.clone(), Pending { original, set });
        }
        let set = &mut self.blocks.get_mut(&block_key).unwrap().set;
        if insert {
            set.insert(id);
        } else {
            set.remove(id);
        }
        Ok(())
    }

    /// Writes buffered bitmap blocks and count deltas into the transaction.
    pub(crate) fn flush(&mut self) -> Result<()> {
        let mut changes: HashMap<(String, String), i64> = HashMap::new();
        for ((field, token, shard), mut block) in self.blocks.drain() {
            *changes.entry((field.clone(), token.clone())).or_default() +=
                block.set.len() as i64 - block.original as i64;
            if block.set.is_empty() {
                self.tx
                    .prepare_cached(
                        "DELETE FROM postings WHERE field=?1 AND token=?2 AND shard=?3",
                    )?
                    .execute(params![field, token, shard])?;
            } else {
                block.set.optimize();
                let mut blob = Vec::with_capacity(block.set.serialized_size());
                block.set.serialize_into(&mut blob)?;
                self.tx.prepare_cached("INSERT INTO postings(field,token,shard,cardinality,bitmap) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(field,token,shard) DO UPDATE SET cardinality=excluded.cardinality,bitmap=excluded.bitmap")?
                    .execute(params![field, token, shard, block.set.len() as i64, blob])?;
            }
        }
        for ((field, token), delta) in changes {
            if delta == 0 {
                continue;
            }
            self.tx.prepare_cached("INSERT INTO counts(field,token,n) VALUES(?1,?2,?3) ON CONFLICT(field,token) DO UPDATE SET n=n+excluded.n")?
                .execute(params![field, token, delta])?;
            self.tx
                .prepare_cached("DELETE FROM counts WHERE field=?1 AND token=?2 AND n=0")?
                .execute(params![field, token])?;
        }
        Ok(())
    }
}
