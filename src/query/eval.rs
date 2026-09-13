use super::Engine;
use super::interval::IntervalOp;
use super::postings::prefix_end;
use crate::index::{EXISTS, LIVE};
use crate::{Direction, NodeSet, Predicate};
use anyhow::Result;
use std::ops::Bound::{Excluded, Included};

impl Engine<'_> {
    /// The Nodes matching `p`, restricted to `candidate` when given.
    pub(crate) fn eval(&self, p: &Predicate, candidate: Option<&NodeSet>) -> Result<NodeSet> {
        if candidate.is_some_and(NodeSet::is_empty) {
            return Ok(NodeSet::new());
        }
        let mut result = match p {
            Predicate::All => {
                return match candidate {
                    Some(s) => Ok(s.clone()),
                    None => self.posting("", LIVE, None),
                };
            }
            Predicate::Ids { ids } => {
                let mut set: NodeSet = ids.iter().copied().collect();
                if let Some(candidate) = candidate {
                    set &= candidate;
                }
                return self.posting("", LIVE, Some(&set));
            }
            Predicate::Eq { field, value } => return self.exact(field, value, candidate),
            Predicate::Exists { field } => return self.posting(field, EXISTS, candidate),
            Predicate::Any { value } => return self.any(value, candidate),
            Predicate::In { field, values } => {
                let mut result = NodeSet::new();
                for value in values {
                    result |= self.exact(field, value, candidate)?;
                }
                result
            }
            Predicate::Prefix { field, prefix } => {
                let start = format!("s:{prefix}");
                let end = prefix_end(&start);
                self.token_range(field, Included(start), Excluded(end), candidate)?
            }
            Predicate::Range {
                field,
                gt,
                gte,
                lt,
                lte,
            } => return self.range(field, [gt, gte, lt, lte], candidate),
            Predicate::At { field, value } => {
                return self.interval(field, IntervalOp::At, value, value, candidate);
            }
            Predicate::Overlap { field, start, end } => {
                return self.interval(field, IntervalOp::Overlap, start, end, candidate);
            }
            Predicate::Contains { field, start, end } => {
                return self.interval(field, IntervalOp::Contains, start, end, candidate);
            }
            Predicate::ContainedBy { field, start, end } => {
                return self.interval(field, IntervalOp::ContainedBy, start, end, candidate);
            }
            Predicate::And { args } => return self.and(args, candidate),
            Predicate::Or { args } => {
                let mut result = NodeSet::new();
                for p in args {
                    result |= self.eval(p, candidate)?;
                }
                result
            }
            Predicate::Not { arg } => {
                let base = match candidate {
                    Some(s) => s.clone(),
                    None => self.posting("", LIVE, None)?,
                };
                let exclude = self.eval(arg, Some(&base))?;
                return Ok(base - exclude);
            }
            Predicate::HasLink {
                relation,
                target,
                direction,
            } => {
                let reverse = match direction {
                    Direction::Out => Direction::In,
                    Direction::In => Direction::Out,
                };
                return self.expand(&NodeSet::from([*target]), relation, reverse, candidate);
            }
            Predicate::Traverse { from, steps } => self.traverse(from, steps, candidate)?,
        };
        if let Some(candidate) = candidate {
            result &= candidate;
        }
        Ok(result)
    }

    // Smallest estimate first; each operand only looks inside the survivors.
    fn and(&self, args: &[Predicate], candidate: Option<&NodeSet>) -> Result<NodeSet> {
        let mut ordered = args
            .iter()
            .map(|p| Ok((self.estimate(p)?, p)))
            .collect::<Result<Vec<_>>>()?;
        ordered.sort_by_key(|(estimate, _)| *estimate);
        let mut current = candidate.cloned();
        for (_, p) in ordered {
            let next = self.eval(p, current.as_ref())?;
            let empty = next.is_empty();
            current = Some(next);
            if empty {
                break;
            }
        }
        match current {
            Some(s) => Ok(s),
            None => self.posting("", LIVE, None),
        }
    }
}
