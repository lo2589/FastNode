use super::range;
use crate::index;
use crate::store::MAX_LINK_LIMIT;
use crate::{Predicate, Query};
use anyhow::{Result, ensure};

pub(super) fn query(query: &Query) -> Result<()> {
    predicate(&query.predicate, 0)?;
    ensure!(
        query.limit <= 100_000,
        "page limit must be <= 100000; paginate with after or cursor"
    );
    ensure!(
        query.link_limit <= MAX_LINK_LIMIT,
        "link_limit must be <= {MAX_LINK_LIMIT}"
    );
    match &query.order_by {
        Some(order) => {
            index::field_valid(&order.field)?;
            ensure!(!order.field.is_empty(), "order_by requires a field");
            ensure!(
                query.after == 0,
                "with order_by, paginate with cursor instead of after"
            );
        }
        None => ensure!(query.cursor.is_none(), "cursor requires order_by"),
    }
    Ok(())
}

pub(super) fn predicate(p: &Predicate, depth: usize) -> Result<()> {
    ensure!(depth < 64, "query nesting must be below 64");
    match p {
        Predicate::Eq { field, value } => {
            index::field_valid(field)?;
            index::token(value)?;
        }
        Predicate::In { field, values } => {
            index::field_valid(field)?;
            for v in values {
                index::token(v)?;
            }
        }
        Predicate::Any { value } => {
            index::token(value)?;
        }
        Predicate::Exists { field } | Predicate::Prefix { field, .. } => index::field_valid(field)?,
        Predicate::Range {
            field,
            gt,
            gte,
            lt,
            lte,
        } => {
            index::field_valid(field)?;
            ensure!(gt.is_none() || gte.is_none(), "choose gt or gte");
            ensure!(lt.is_none() || lte.is_none(), "choose lt or lte");
            range::kind([gt, gte, lt, lte])?;
        }
        Predicate::At { field, value } => {
            index::field_valid(field)?;
            index::number_key(value)?;
        }
        Predicate::Overlap { field, start, end }
        | Predicate::Contains { field, start, end }
        | Predicate::ContainedBy { field, start, end } => {
            index::field_valid(field)?;
            ensure!(
                index::number_key(start)? <= index::number_key(end)?,
                "interval start must not exceed end"
            );
        }
        Predicate::Seek {
            index: name,
            group,
            gt,
            gte,
            lt,
            lte,
            limit,
            ..
        } => {
            ensure!(!name.is_empty(), "seek requires an index name");
            ensure!(!group.is_empty(), "seek requires group values");
            for value in group {
                index::token(value)?;
            }
            ensure!(gt.is_none() || gte.is_none(), "choose gt or gte");
            ensure!(lt.is_none() || lte.is_none(), "choose lt or lte");
            range::kind([gt, gte, lt, lte])?;
            ensure!(
                limit.is_none_or(|l| l <= 100_000),
                "seek limit must be <= 100000"
            );
        }
        Predicate::And { args } | Predicate::Or { args } => {
            for p in args {
                predicate(p, depth + 1)?;
            }
        }
        Predicate::Not { arg } => predicate(arg, depth + 1)?,
        Predicate::Traverse { from, steps } => {
            predicate(from, depth + 1)?;
            ensure!(
                steps.len() <= 64,
                "a traversal supports up to 64 explicit steps"
            );
            for step in steps {
                ensure!(!step.relation.is_empty(), "relation must not be empty");
                if let Some((min, Some(max))) = step.repeat() {
                    ensure!(min <= max, "step min must not exceed max");
                }
                if let Some(filter) = &step.filter {
                    predicate(filter, depth + 1)?;
                }
            }
        }
        Predicate::HasLink { relation, .. } => {
            ensure!(!relation.is_empty(), "relation must not be empty")
        }
        Predicate::All | Predicate::Ids { .. } => {}
    }
    Ok(())
}
