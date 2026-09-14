use super::Write;
use anyhow::Result;
use rusqlite::OptionalExtension;
use std::time::{SystemTime, UNIX_EPOCH};

impl Write<'_> {
    /// This transaction's timestamp in UTC milliseconds. It stays fixed for
    /// the whole transaction and is strictly greater than every earlier
    /// committed transaction's, even if the system clock steps back.
    pub fn now(&mut self) -> Result<i64> {
        if let Some(now) = self.now {
            return Ok(now);
        }
        let last: Option<i64> = self
            .tx
            .query_row("SELECT last FROM clock WHERE id=1", [], |r| r.get(0))
            .optional()?;
        let system = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
        let now = last.map_or(system, |last| system.max(last + 1));
        self.tx.execute(
            "INSERT INTO clock(id,last) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET last=excluded.last",
            [now],
        )?;
        self.now = Some(now);
        Ok(now)
    }
}
