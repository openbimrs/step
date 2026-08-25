//! Physical-file identification and preamble handling.

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
    bytes[start..].starts_with(b"ISO-10303-21")
}
