//! Cheap bounds used to evaluate the smallest AND operand first. A
//! single-holder number has no count row and estimates as 0, which is at
//! most one off.

use super::Engine;
use crate::index::{self, EXISTS, LIVE};
use crate::{Direction, Predicate};
use anyhow::Result;
use rusqlite::params;

impl Engine<'_> {
    pub(super) fn estimate(&self, p: &Predicate) -> Result<u64> {
        Ok(match p {
            Predicate::Eq { field, value } => self.count(field, &index::token(value)?.0)?,
            Predicate::Exists { field } => self.count(field, EXISTS)?,
            Predicate::Ids { ids } => ids.len() as u64,
            Predicate::Any { value } => self
                .any_fields(value)?
                .iter()
                .fold(0u64, |sum, (_, n)| sum.saturating_add(*n)),
            Predicate::In { field, values } => {
                let mut sum = 0u64;
                for value in values {
                    sum = sum.saturating_add(self.count(field, &index::token(value)?.0)?);
                }
                sum
            }
            Predicate::And { args } => {
                let mut min = self.count("", LIVE)?;
                for p in args {
                    min = min.min(self.estimate(p)?);
                }
                min
            }
            Predicate::Or { args } => {
                let mut sum = 0u64;
                for p in args {
                    sum = sum.saturating_add(self.estimate(p)?);
                }
                sum
            }
            Predicate::HasLink {
                relation,
                target,
                direction,
            } => {
                let sql = match direction {
                    Direction::Out => "SELECT count(*) FROM links WHERE target=?1 AND relation=?2",
                    Direction::In => "SELECT count(*) FROM links WHERE source=?1 AND relation=?2",
                };
                self.conn
                    .prepare_cached(sql)?
                    .query_row(params![target, relation], |r| r.get::<_, i64>(0))?
                    as u64
            }
            Predicate::Seek {
                limit: Some(limit), ..
            } => *limit as u64,
            _ => self.count("", LIVE)?,
        })
    }
}
