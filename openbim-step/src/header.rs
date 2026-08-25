//! Physical-file identification and preamble handling.

const MARKER: &[u8] = b"ISO-10303-21";

/// Returns whether bytes begin with the ISO 10303-21 marker.
///
/// A UTF-8 BOM and leading ASCII whitespace are accepted because both are
/// emitted by real-world authoring tools.
#[must_use]
pub fn is_step_file(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    bytes[start..]
        .iter()
        .copied()
        .filter(|byte| !matches!(byte, b'\t' | b'\n' | b'\r' | 0x0c))
        .take(MARKER.len())
        .eq(MARKER.iter().copied())
}
