use anyhow::{Result, bail, ensure};

pub(crate) const TYPE_FIELD: &str = "@type";

// Fields are JSON Pointers into attrs, plus @type. A segment `*` means any
// array index; an object key that is literally `*` is written `~2`.
pub(crate) fn field_valid(field: &str) -> Result<()> {
    if field == TYPE_FIELD {
        return Ok(());
    }
    ensure!(
        field.is_empty() || field.starts_with('/'),
        "field must be a JSON Pointer such as /city or /jobs/*/company, or @type"
    );
    for segment in field.split('/').skip(1) {
        let mut chars = segment.chars();
        while let Some(ch) = chars.next() {
            if ch == '~' {
                match chars.next() {
                    Some('0' | '1') => {}
                    Some('2') => ensure!(
                        segment == "~2",
                        "~2 must be a whole segment; it names the literal key *"
                    ),
                    _ => bail!("invalid JSON Pointer escape"),
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn is_wildcard(field: &str) -> bool {
    field.split('/').any(|segment| segment == "*")
}

pub(super) fn escape(key: &str) -> String {
    if key == "*" {
        "~2".into()
    } else {
        key.replace('~', "~0").replace('/', "~1")
    }
}
