//! Minimal UTF-8 decoder (no_std).
//!
//! Provides a streaming-style decoder: given a byte slice it yields each
//! Unicode scalar value along with the number of bytes it occupies. Invalid
//! sequences are reported as [`core::char::REPLACEMENT_CHARACTER`] so callers
//! never panic on bad input.

/// Decode the first UTF-8 scalar value at the start of `bytes`.
///
/// Returns `(character, consumed)` where `consumed` is the number of bytes the
/// scalar occupied (always at least 1). On truncated or malformed input the
/// replacement character is returned and `consumed` is advanced past the bad
/// lead byte so decoding can continue.
pub fn decode_one(bytes: &[u8]) -> (char, usize) {
    let repl = ('\u{FFFD}', 1);
    let Some(&b0) = bytes.first() else {
        return ('\u{FFFD}', 0);
    };

    // Fast path: ASCII.
    if b0 < 0x80 {
        return (b0 as char, 1);
    }

    // Determine sequence length from the lead byte.
    let len = if b0 & 0xE0 == 0xC0 {
        2
    } else if b0 & 0xF0 == 0xE0 {
        3
    } else if b0 & 0xF8 == 0xF0 {
        4
    } else {
        // Continuation byte without a lead, or an invalid lead.
        return repl;
    };

    if bytes.len() < len {
        return repl; // truncated
    }

    // Every continuation byte must be 10xxxxxx.
    for b in &bytes[1..len] {
        if b & 0xC0 != 0x80 {
            return repl;
        }
    }

    let cp = match len {
        2 => {
            let v = ((b0 as u32 & 0x1F) << 6) | (bytes[1] as u32 & 0x3F);
            // Overlong encoding check.
            if v < 0x80 {
                return repl;
            }
            v
        }
        3 => {
            let v = ((b0 as u32 & 0x0F) << 12)
                | ((bytes[1] as u32 & 0x3F) << 6)
                | (bytes[2] as u32 & 0x3F);
            if v < 0x800 || (0xD800..=0xDFFF).contains(&v) {
                return repl; // overlong or surrogate
            }
            v
        }
        _ => {
            let v = ((b0 as u32 & 0x07) << 18)
                | ((bytes[1] as u32 & 0x3F) << 12)
                | ((bytes[2] as u32 & 0x3F) << 6)
                | (bytes[3] as u32 & 0x3F);
            if v < 0x10000 || v > 0x10FFFF {
                return repl; // overlong or out of range
            }
            v
        }
    };

    match char::from_u32(cp) {
        Some(c) => (c, len),
        None => repl,
    }
}

/// Iterator over the scalar values of a UTF-8 byte slice. Invalid bytes are
/// yielded as the replacement character rather than stopping iteration.
pub struct Utf8Chars<'a> {
    bytes: &'a [u8],
}

impl<'a> Utf8Chars<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Utf8Chars { bytes }
    }
}

impl<'a> Iterator for Utf8Chars<'a> {
    type Item = char;
    fn next(&mut self) -> Option<char> {
        if self.bytes.is_empty() {
            return None;
        }
        let (c, n) = decode_one(self.bytes);
        let n = n.max(1);
        self.bytes = &self.bytes[n.min(self.bytes.len())..];
        Some(c)
    }
}

/// Convenience: count scalar values in a UTF-8 slice (invalid bytes count too).
pub fn count_chars(bytes: &[u8]) -> usize {
    Utf8Chars::new(bytes).count()
}
