use super::Engine;
use crate::{Direction, NodeId, NodeSet, Predicate, Step};
use anyhow::Result;
use rusqlite::params;

impl Engine<'_> {
    pub(super) fn traverse(
        &self,
        from: &Predicate,
        steps: &[Step],
        candidate: Option<&NodeSet>,
    ) -> Result<NodeSet> {
        // Intermediate Nodes are in a different domain from the final
        // candidate set. Only push final candidates into the final hop.
        let mut current = self.eval(from, None)?;
        for (i, step) in steps.iter().enumerate() {
            let last = i + 1 == steps.len();
            let target_candidate = if last { candidate } else { None };
            current = match step.repeat() {
                None => {
                    let targets = match &step.filter {
                        Some(filter) => Some(self.eval(filter, target_candidate)?),
                        None => target_candidate.cloned(),
                    };
                    self.expand(&current, &step.relation, step.direction, targets.as_ref())?
                }
                // Intermediate hops pass through every Node; the filter only
                // selects which reached Nodes are kept.
                Some((min, max)) => {
                    let mut reached =
                        self.walk(&current, &step.relation, step.direction, min, max)?;
                    if let Some(candidate) = target_candidate {
                        reached &= candidate;
                    }
                    match &step.filter {
                        Some(filter) => self.eval(filter, Some(&reached))?,
                        None => reached,
                    }
                }
            };
            if current.is_empty() {
                break;
            }
        }
        Ok(current)
    }

    // Breadth-first by hop count. A Node counts at its shortest distance from
    // the start set, so cycles terminate once no unvisited Node remains.
    fn walk(
        &self,
        start: &NodeSet,
        relation: &str,
        direction: Direction,
        min: u32,
        max: Option<u32>,
    ) -> Result<NodeSet> {
        let mut visited = start.clone();
        let mut frontier = start.clone();
        let mut reached = if min == 0 {
            start.clone()
        } else {
            NodeSet::new()
        };
        let mut depth = 0u32;
        while !frontier.is_empty() && max.is_none_or(|max| depth < max) {
            depth += 1;
            frontier = self.expand(&frontier, relation, direction, None)? - &visited;
            visited |= &frontier;
            if depth >= min {
                reached |= &frontier;
            }
        }
        Ok(reached)
    }

    pub(super) fn expand(
        &self,
        sources: &NodeSet,
        relation: &str,
        direction: Direction,
        targets: Option<&NodeSet>,
    ) -> Result<NodeSet> {
        let mut result = NodeSet::new();
        // Walk from whichever side is smaller.
        let flipped = targets.is_some_and(|t| t.len() < sources.len());
        let (input, check, dir) = if flipped {
            let reverse = match direction {
                Direction::Out => Direction::In,
                Direction::In => Direction::Out,
            };
            (targets.unwrap(), Some(sources), reverse)
        } else {
            (sources, targets, direction)
        };
        let sql = match dir {
            Direction::Out => "SELECT target FROM links WHERE source=?1 AND relation=?2",
            Direction::In => "SELECT source FROM links WHERE target=?1 AND relation=?2",
        };
        let mut stmt = self.conn.prepare_cached(sql)?;
        for id in input {
            let mut rows = stmt.query(params![id, relation])?;
            while let Some(row) = rows.next()? {
                let other: NodeId = row.get(0)?;
                if check.is_none_or(|s| s.contains(other)) {
                    result.insert(if flipped { id } else { other });
                    if flipped {
                        break;
                    }
                }
            }
        }
        Ok(result)
    }
}
