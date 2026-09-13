use crate::{NewNode, Store};
use anyhow::{Context, Result, ensure};
use serde::{
    Serialize,
    de::{self, DeserializeSeed, SeqAccess, Visitor},
};
use serde_json::Value;
use std::{
    fmt,
    io::{BufRead, BufReader, Read},
};

#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    pub imported: u64,
    pub batches: u64,
    pub first_id: Option<u32>,
    pub last_id: Option<u32>,
}

struct Sink<'a> {
    store: &'a mut Store,
    batch: Vec<NewNode>,
    size: usize,
    report: ImportReport,
}
impl Sink<'_> {
    fn push(&mut self, value: Value) -> Result<()> {
        let record = self.report.imported + self.batch.len() as u64 + 1;
        let node = serde_json::from_value(value)
            .with_context(|| format!("record {record} is not a Node {{type, summary, attrs}}"))?;
        self.batch.push(node);
        if self.batch.len() == self.size {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        if self.batch.is_empty() {
            return Ok(());
        }
        let ids = self.store.create_many(std::mem::take(&mut self.batch))?;
        self.report.imported += ids.len() as u64;
        self.report.batches += 1;
        if self.report.first_id.is_none() {
            self.report.first_id = ids.first().copied();
        }
        self.report.last_id = ids.last().copied();
        Ok(())
    }
}
struct ArraySeed<'a, 'b>(&'a mut Sink<'b>);
impl<'de> DeserializeSeed<'de> for ArraySeed<'_, '_> {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> std::result::Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for ArraySeed<'_, '_> {
    type Value = ();
    fn expecting(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        write!(fmt, "an array of JSON objects")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
        while let Some(value) = seq.next_element::<Value>()? {
            self.0.push(value).map_err(de::Error::custom)?;
        }
        Ok(())
    }
}

impl Store {
    /// Streams an array, JSONL, or whitespace-separated JSON objects. Each
    /// batch commits atomically. Errors report the already committed count.
    pub fn import(&mut self, reader: impl Read, batch_size: usize) -> Result<ImportReport> {
        ensure!(batch_size > 0, "batch_size must be positive");
        let mut reader = BufReader::new(reader);
        let first = loop {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                break None;
            }
            if let Some(i) = buffer.iter().position(|b| !b.is_ascii_whitespace()) {
                let first = buffer[i];
                reader.consume(i);
                break Some(first);
            }
            let len = buffer.len();
            reader.consume(len);
        };
        let mut sink = Sink {
            store: self,
            batch: Vec::new(),
            size: batch_size,
            report: ImportReport::default(),
        };
        let result = (|| -> Result<()> {
            if first == Some(b'[') {
                let mut deserializer = serde_json::Deserializer::from_reader(reader);
                ArraySeed(&mut sink).deserialize(&mut deserializer)?;
                deserializer.end()?;
            } else {
                for value in serde_json::Deserializer::from_reader(reader).into_iter::<Value>() {
                    sink.push(value?)?;
                }
            }
            sink.flush()
        })();
        result.with_context(|| {
            format!(
                "import failed; {} nodes in {} batches already committed; last_id={:?}",
                sink.report.imported, sink.report.batches, sink.report.last_id
            )
        })?;
        Ok(sink.report)
    }
}
