//! Sort keys and the opaque cursor that resumes an ordered query.

use crate::index;
use crate::{NodeId, SortDirection};
use anyhow::{Context, Result, ensure};
use std::cmp::Ordering;

pub(super) const NUMBER: u8 = 0;
pub(super) const STRING: u8 = 1;
pub(super) const MISSING: u8 = 2;

/// A Node's place in an ordered result: value kind, value bytes, then id.
#[derive(Debug, Clone)]
pub(super) struct Key {
    pub rank: u8,
    pub bytes: Vec<u8>,
    pub id: NodeId,
}

/// Numbers precede strings ascending and follow them descending; Nodes
/// without a value always come last.
pub(super) fn position(rank: u8, direction: SortDirection) -> u8 {
    match (rank, direction) {
        (MISSING, _) => 2,
        (rank, SortDirection::Asc) => rank,
        (rank, SortDirection::Desc) => 1 - rank,
    }
}

/// Values compare by bytes then id, reversed when descending; Nodes without a
/// value stay in ascending id order.
pub(super) fn compare(a: &Key, b: &Key, direction: SortDirection) -> Ordering {
    position(a.rank, direction)
        .cmp(&position(b.rank, direction))
        .then_with(|| {
            let ascending = (&a.bytes, a.id).cmp(&(&b.bytes, b.id));
            if a.rank == MISSING || direction == SortDirection::Asc {
                ascending
            } else {
                ascending.reverse()
            }
        })
}

/// The posting token holding this key's value.
pub(super) fn token(key: &Key) -> Result<String> {
    Ok(match key.rank {
        NUMBER => index::number_token(&key.bytes),
        _ => format!("s:{}", std::str::from_utf8(&key.bytes)?),
    })
}

pub(super) fn encode(key: &Key) -> String {
    format!("{}.{}.{}", key.rank, index::hex(&key.bytes), key.id)
}

pub(super) fn decode(cursor: &str) -> Result<Key> {
    let invalid = || format!("invalid cursor {cursor}");
    let mut parts = cursor.split('.');
    let rank: u8 = parts
        .next()
        .with_context(invalid)?
        .parse()
        .with_context(invalid)?;
    let bytes = index::unhex(parts.next().with_context(invalid)?).with_context(invalid)?;
    let id: NodeId = parts
        .next()
        .with_context(invalid)?
        .parse()
        .with_context(invalid)?;
    ensure!(rank <= MISSING && parts.next().is_none(), invalid());
    if rank == STRING {
        std::str::from_utf8(&bytes).with_context(invalid)?;
    }
    Ok(Key { rank, bytes, id })
}
