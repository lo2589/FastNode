use anyhow::{Context, Result, bail, ensure};
use serde_json::{Number, Value};

/// Sorts after every number key; stored as the end of an open interval.
pub(crate) const INFINITY: &[u8] = &[3];

// Lexicographically sortable arbitrary-precision decimal. This avoids f64
// rounding of JSON integers beyond 2^53, including full u64 values.
pub(crate) fn number_key(number: &Number) -> Result<Vec<u8>> {
    let raw = number.to_string();
    let negative = raw.starts_with('-');
    let unsigned = raw.trim_start_matches('-');
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(i) => (&unsigned[..i], unsigned[i + 1..].parse::<i64>()?),
        None => (unsigned, 0),
    };
    let integer_len = mantissa.find('.').unwrap_or(mantissa.len()) as i64;
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let leading = digits.bytes().take_while(|c| *c == b'0').count();
    let significand = digits[leading..].trim_end_matches('0');
    if significand.is_empty() {
        return Ok(vec![1]);
    }
    let magnitude = exponent
        .checked_add(integer_len)
        .and_then(|x| x.checked_sub(leading as i64))
        .ok_or_else(|| anyhow::anyhow!("numeric exponent exceeds supported i64 magnitude"))?;
    let mut key = vec![if negative { 0 } else { 2 }];
    let sortable_exponent = (magnitude as u64) ^ (1 << 63);
    key.extend_from_slice(&sortable_exponent.to_be_bytes());
    key.extend_from_slice(significand.as_bytes());
    key.push(0);
    if negative {
        for byte in &mut key[1..] {
            *byte = !*byte;
        }
    }
    Ok(key)
}

/// The f64 nearest a number key: 0.digits × 10^magnitude. None for INFINITY
/// or a value outside f64's range.
pub(crate) fn key_to_f64(key: &[u8]) -> Option<f64> {
    let negative = match key.first()? {
        0 => true,
        1 => return Some(0.0),
        2 => false,
        _ => return None,
    };
    let bytes: Vec<u8> = key[1..]
        .iter()
        .map(|b| if negative { !b } else { *b })
        .collect();
    let magnitude = (u64::from_be_bytes(bytes.get(..8)?.try_into().ok()?) ^ (1 << 63)) as i64;
    let digits = std::str::from_utf8(bytes.get(8..bytes.len().checked_sub(1)?)?).ok()?;
    let sign = if negative { "-" } else { "" };
    let value: f64 = format!("{sign}0.{digits}e{magnitude}").parse().ok()?;
    value.is_finite().then_some(value)
}

/// An upper bound on `hi - lo` with room for f64 rounding; None when either
/// end is outside f64's range.
pub(crate) fn span_length(lo: &[u8], hi: &[u8]) -> Option<f64> {
    let (lo, hi) = (key_to_f64(lo)?, key_to_f64(hi)?);
    let length = (hi - lo).max(0.0) + (hi.abs() + lo.abs()) * 1e-12 + f64::MIN_POSITIVE;
    length.is_finite().then_some(length)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

pub(crate) fn unhex(text: &str) -> Result<Vec<u8>> {
    ensure!(text.len().is_multiple_of(2), "odd hex length");
    (0..text.len())
        .step_by(2)
        .map(|i| {
            text.get(i..i + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .context("invalid hex")
        })
        .collect()
}

/// Posting tokens keep value order: hex keeps number-key byte order.
pub(crate) fn number_token(key: &[u8]) -> String {
    format!("n:{}", hex(key))
}

/// The bytes a sortable token orders by: the number key or the UTF-8 string.
pub(crate) fn token_sort_bytes(token: &str) -> Result<Vec<u8>> {
    if let Some(digits) = token.strip_prefix("n:") {
        unhex(digits)
    } else if let Some(text) = token.strip_prefix("s:") {
        Ok(text.as_bytes().to_vec())
    } else {
        bail!("token {token} is not sortable")
    }
}

pub(crate) fn token(value: &Value) -> Result<(String, Option<Vec<u8>>)> {
    Ok(match value {
        Value::Null => ("z:".into(), None),
        Value::Bool(b) => (format!("b:{}", u8::from(*b)), None),
        Value::String(s) => (format!("s:{s}"), None),
        Value::Number(n) => {
            let key = number_key(n)?;
            (number_token(&key), Some(key))
        }
        _ => bail!("Eq/In accept scalar values; query array membership with a scalar value"),
    })
}
