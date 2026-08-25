//! Physical-file identification and preamble handling.

const MARKER: &[u8] = b"ISO-10303-21";

const fn is_line_delimiter(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | b'\r' | 0x0c)
}

fn match_ignoring_line_delimiters(bytes: &[u8], start: usize, expected: &[u8]) -> Option<usize> {
    let mut position = start;
    for &expected_byte in expected {
        while bytes
            .get(position)
            .is_some_and(|byte| is_line_delimiter(*byte))
        {
            position += 1;
        }
        if bytes.get(position) != Some(&expected_byte) {
            return None;
        }
        position += 1;
    }
    Some(position)
}

fn skip_leading_separators(bytes: &[u8], mut position: usize) -> usize {
    loop {
        while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
            position += 1;
        }
        let Some(end) = match_ignoring_line_delimiters(bytes, position, b"\\N\\")
            .or_else(|| match_ignoring_line_delimiters(bytes, position, b"\\F\\"))
        else {
            return position;
        };
        position = end;
    }
}

/// Returns whether bytes begin with the ISO 10303-21 marker.
///
/// A UTF-8 BOM and leading ASCII whitespace are accepted because both are
/// emitted by real-world authoring tools.
#[must_use]
pub fn is_step_file(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let start = skip_leading_separators(bytes, 0);
    match_ignoring_line_delimiters(bytes, start, MARKER).is_some()
}
