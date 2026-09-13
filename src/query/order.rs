//! ORDER BY over one single-valued path, without loading JSON:
//! - few candidates: look up each Node's key, sort in memory;
//! - numbers: walk the ordered number rows;
//! - strings: walk value bitmaps in token order;
//! - Nodes without a sortable value: candidates minus the path's `o:` bitmap.

use super::Engine;
use super::order_key::{self, Key, MISSING, NUMBER, STRING};
use crate::index::{self, MULTI, ORDERED};
use crate::store::get_node;
use crate::{NodeId, NodeSet, OrderBy, SortDirection};
use anyhow::{Result, ensure};
use rusqlite::{OptionalExtension, params};
use std::ops::Bound::{Excluded, Unbounded};

const PROBE_LIMIT: u64 = 2048;

pub(super) struct Page {
    pub ids: Vec<NodeId>,
    pub next_cursor: Option<String>,
}

// Receives keys in sort order and keeps the ones after the cursor.
struct Collector<'a> {
    set: &'a NodeSet,
    after: Option<&'a Key>,
    direction: SortDirection,
    want: usize,
    keys: Vec<Key>,
}

impl Collector<'_> {
    fn full(&self) -> bool {
        self.keys.len() >= self.want
    }
    fn offer(&mut self, key: Key) {
        let after_cursor = self
            .after
            .is_none_or(|a| order_key::compare(&key, a, self.direction).is_gt());
        if after_cursor && self.set.contains(key.id) {
            self.keys.push(key);
        }
    }
}

impl Engine<'_> {
    pub(super) fn order(
        &self,
        set: &NodeSet,
        order: &OrderBy,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<Page> {
        let field = order.field.as_str();
        ensure!(
            self.count(field, MULTI)? == 0,
            "{field} holds several values in some Nodes; order by a single-valued path"
        );
        let after = cursor.map(order_key::decode).transpose()?;
        let mut out = Collector {
            set,
            after: after.as_ref(),
            direction: order.direction,
            want: limit.saturating_add(1),
            keys: Vec::new(),
        };
        if set.len() <= PROBE_LIMIT {
            self.probe(field, &mut out)?;
        } else {
            self.stream(field, &mut out)?;
        }
        let more = out.keys.len() > limit;
        out.keys.truncate(limit);
        Ok(Page {
            ids: out.keys.iter().map(|k| k.id).collect(),
            next_cursor: more
                .then(|| out.keys.last().map(order_key::encode))
                .flatten(),
        })
    }

    fn probe(&self, field: &str, out: &mut Collector) -> Result<()> {
        let strings = self.has_tokens(field, "s:", "s;")?;
        let mut keys = Vec::with_capacity(out.set.len() as usize);
        for id in out.set {
            keys.push(self.sort_key(field, id, strings)?);
        }
        keys.sort_by(|a, b| order_key::compare(a, b, out.direction));
        for key in keys {
            if out.full() {
                break;
            }
            out.offer(key);
        }
        Ok(())
    }

    fn sort_key(&self, field: &str, id: NodeId, strings: bool) -> Result<Key> {
        let number: Option<Vec<u8>> = self
            .conn
            .prepare_cached("SELECT value FROM numbers WHERE node_id=?1 AND field=?2 LIMIT 1")?
            .query_row(params![id, field], |r| r.get(0))
            .optional()?;
        if let Some(bytes) = number {
            return Ok(Key {
                rank: NUMBER,
                bytes,
                id,
            });
        }
        // Strings have no per-Node index; derive this Node's postings.
        if strings && let Some(node) = get_node(self.conn, id)? {
            let attrs = index::attributes(&node.kind, &node.attrs)?;
            let range = (field.to_owned(), "s:".to_owned())..(field.to_owned(), "s;".to_owned());
            if let Some(((_, token), _)) = attrs.postings.range(range).next() {
                let bytes = index::token_sort_bytes(token)?;
                return Ok(Key {
                    rank: STRING,
                    bytes,
                    id,
                });
            }
        }
        Ok(Key {
            rank: MISSING,
            bytes: Vec::new(),
            id,
        })
    }

    fn stream(&self, field: &str, out: &mut Collector) -> Result<()> {
        let sections = match out.direction {
            SortDirection::Asc => [NUMBER, STRING, MISSING],
            SortDirection::Desc => [STRING, NUMBER, MISSING],
        };
        for rank in sections {
            let before_cursor = out.after.is_some_and(|a| {
                order_key::position(rank, out.direction)
                    < order_key::position(a.rank, out.direction)
            });
            if out.full() || before_cursor {
                continue;
            }
            match rank {
                NUMBER => self.number_rows(field, out)?,
                STRING => self.string_bitmaps(field, out)?,
                _ => self.missing(field, out)?,
            }
        }
        Ok(())
    }

    // String tokens sort by UTF-8 bytes; each value's bitmap is in id order.
    fn string_bitmaps(&self, field: &str, out: &mut Collector) -> Result<()> {
        let (mut lo, mut hi) = ("s:".to_owned(), "s;".to_owned());
        let sql = match out.direction {
            SortDirection::Asc => {
                "SELECT token,bitmap FROM postings WHERE field=?1 AND token>=?2 AND token<?3 ORDER BY token,shard"
            }
            SortDirection::Desc => {
                "SELECT token,bitmap FROM postings WHERE field=?1 AND token>=?2 AND token<=?3 ORDER BY token DESC,shard DESC"
            }
        };
        // Resume at the cursor's own value; offer skips ids up to the cursor.
        if let Some(after) = out.after.filter(|a| a.rank == STRING) {
            match out.direction {
                SortDirection::Asc => lo = order_key::token(after)?,
                SortDirection::Desc => hi = order_key::token(after)?,
            }
        }
        let mut stmt = self.conn.prepare_cached(sql)?;
        let mut rows = stmt.query(params![field, lo, hi])?;
        while let Some(row) = rows.next()? {
            if out.full() {
                break;
            }
            let blob: Vec<u8> = row.get(1)?;
            let block = NodeSet::deserialize_from(&blob[..])? & out.set;
            if block.is_empty() {
                continue;
            }
            let bytes = index::token_sort_bytes(&row.get::<_, String>(0)?)?;
            let ids: Box<dyn Iterator<Item = NodeId>> = match out.direction {
                SortDirection::Asc => Box::new(block.iter()),
                SortDirection::Desc => Box::new(block.iter().rev()),
            };
            for id in ids {
                out.offer(Key {
                    rank: STRING,
                    bytes: bytes.clone(),
                    id,
                });
                if out.full() {
                    break;
                }
            }
        }
        Ok(())
    }

    fn number_rows(&self, field: &str, out: &mut Collector) -> Result<()> {
        let (sql, start) = match out.direction {
            SortDirection::Asc => (
                "SELECT value,node_id FROM numbers WHERE field=?1 AND (value,node_id)>(?2,?3) ORDER BY value,node_id",
                (Vec::new(), 0),
            ),
            SortDirection::Desc => (
                "SELECT value,node_id FROM numbers WHERE field=?1 AND (value,node_id)<(?2,?3) ORDER BY value DESC,node_id DESC",
                (vec![0xFF], i64::MAX),
            ),
        };
        let (bytes, id) = match out.after.filter(|a| a.rank == NUMBER) {
            Some(after) => (after.bytes.clone(), i64::from(after.id)),
            None => start,
        };
        let mut stmt = self.conn.prepare_cached(sql)?;
        let mut rows = stmt.query(params![field, bytes, id])?;
        while let Some(row) = rows.next()? {
            if out.full() {
                break;
            }
            let id: NodeId = row.get(1)?;
            if out.set.contains(id) {
                out.offer(Key {
                    rank: NUMBER,
                    bytes: row.get(0)?,
                    id,
                });
            }
        }
        Ok(())
    }

    fn missing(&self, field: &str, out: &mut Collector) -> Result<()> {
        let present = self.posting(field, ORDERED, Some(out.set))?;
        let after = out.after.filter(|a| a.rank == MISSING).map_or(0, |a| a.id);
        let absent = out.set - present;
        for id in absent.range((Excluded(after), Unbounded)) {
            if out.full() {
                break;
            }
            out.offer(Key {
                rank: MISSING,
                bytes: Vec::new(),
                id,
            });
        }
        Ok(())
    }
}
