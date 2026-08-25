//! Record-aligned data partitioning.
//!
//! Arbitrary byte splitting is unsafe: a target offset can land inside a
//! quoted string, comment, aggregate, or record. This module tokenizes first,
//! identifies complete `#id=...;` records, and only emits boundaries between
//! those records.

use crate::lexer::{Lexer, Token};
use crate::{Span, StepError};

/// A half-open byte range containing one or more complete data records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Partition {
    /// Inclusive offset aligned to a `#id=` record start.
    pub start: usize,
    /// Exclusive offset aligned to the next record start or final semicolon.
    pub end: usize,
}

impl Partition {
    /// Converts this partition to a source span.
    #[must_use]
    pub const fn span(self) -> Span {
        Span::new(self.start, self.end)
    }
}

/// Locates complete data records.
///
/// For a complete exchange only records inside `DATA; ... ENDSEC;` are
/// returned. For a record-only partition (with no `DATA` marker), all top-level
/// `#id=...;` records are returned. Returned spans are relative to `input`.
/// # Errors
///
/// Returns a lexical diagnostic when the input contains malformed tokens.
pub fn data_record_spans(input: &[u8]) -> Result<Vec<Span>, StepError> {
    let tokens = Lexer::new(input).collect::<Result<Vec<_>, _>>()?;
    let has_data = tokens.windows(2).any(|window| {
        matches!(&window[0].value, Token::Name(name) if name.eq_ignore_ascii_case(b"DATA"))
            && window[1].value == Token::Semicolon
    });
    let mut in_data = !has_data;
    let mut active_start = None;
    let mut records = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        let token = &tokens[index];
        if let Token::Name(name) = &token.value {
            if has_data && name.eq_ignore_ascii_case(b"DATA") && active_start.is_none() {
                in_data = true;
                index += 1;
                continue;
            }
            if has_data && name.eq_ignore_ascii_case(b"ENDSEC") && active_start.is_none() {
                in_data = false;
                index += 1;
                continue;
            }
        }

        if in_data && active_start.is_none() {
            if matches!(token.value, Token::Id(_))
                && tokens
                    .get(index + 1)
                    .is_some_and(|next| next.value == Token::Equals)
            {
                active_start = Some(token.span.start);
            }
        } else if let (Some(start), Token::Semicolon) = (active_start, &token.value) {
            records.push(Span::new(start, token.span.end));
            active_start = None;
        }
        index += 1;
    }

    if let Some(start) = active_start {
        return Err(StepError::syntax(
            Span::new(start, input.len()),
            "unterminated data record",
        ));
    }
    Ok(records)
}

/// Splits data records into at most `partition_count` balanced groups.
///
/// Every partition starts at a record start; every non-final end is the next
/// partition's record start. Inter-record whitespace and comments are assigned
/// to the preceding partition. Empty partitions are never returned.
/// # Errors
///
/// Returns a diagnostic when `partition_count` is zero or tokenization fails.
pub fn partition_data_records(
    input: &[u8],
    partition_count: usize,
) -> Result<Vec<Partition>, StepError> {
    if partition_count == 0 {
        return Err(StepError::invalid_argument(
            "partition_count must be greater than zero",
        ));
    }
    let records = data_record_spans(input)?;
    if records.is_empty() {
        return Ok(Vec::new());
    }
    let count = partition_count.min(records.len());
    let mut starts = Vec::with_capacity(count + 1);
    let base_size = records.len() / count;
    let remainder = records.len() % count;
    for partition in 0..count {
        // The first `remainder` groups receive one extra record.
        let record_index = partition * base_size + partition.min(remainder);
        starts.push(records[record_index].start);
    }
    starts.push(records[records.len() - 1].end);

    Ok(starts
        .windows(2)
        .map(|window| Partition {
            start: window[0],
            end: window[1],
        })
        .collect())
}
