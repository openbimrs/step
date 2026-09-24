//! Parallel parsing with a result identical to [`parse_with`].
//!
//! The data section is split at byte offsets that look like record
//! boundaries (`;` then optional whitespace then `#` and a digit), and each
//! slice is parsed on its own thread.
//!
//! Correctness does not rest on that guess. Between two records, the
//! sequential parser's entire state is its byte offset: phase `DATA`, no
//! lookahead, the lexer just past a `;`. Each slice starts at the offset
//! where the previous slice's parser *actually* stopped, and a slice only
//! counts if its parser lands exactly on its end offset. If every slice
//! does, each one saw precisely the state the sequential parser would have
//! had there, so the concatenation is the sequential result. If any slice
//! overshoots (the guess fell inside a string, comment, or a damaged record
//! that recovery skips past) or fails, the attempt is discarded and the
//! file is parsed sequentially. Errors therefore always come from the
//! sequential parser and are identical by construction.
//!
//! Diagnostics are reproduced in the sequential order: slices and their
//! diagnostics are concatenated in source order, and the reference check
//! runs once over the merged records, so references across slices resolve
//! and its final positional sort matches the sequential one.

use crate::parser::{parse_chunk, parse_prefix, Chunk};
use crate::references::ReferenceCheck;
use crate::{parse_with, DataSection, Exchange, ParseOptions, ParseOutcome, StepError};

/// Parses on up to `threads` threads, returning exactly what
/// [`parse_with`] returns for the same input and options: the same
/// exchange, the same diagnostics in the same order, the same error.
///
/// `threads <= 1` parses sequentially. Splitting pays off on large inputs
/// only -- each thread starts with a fresh allocator arena and the merge is
/// a move per record -- so callers should parse small files (below a few
/// megabytes) with [`parse_with`] directly.
///
/// Worst case: when a split point turns out not to be a record boundary
/// for the parser, the parallel work is discarded and the file is parsed
/// again sequentially, costing up to one extra sequential parse.
/// # Errors
///
/// Exactly the errors of [`parse_with`].
pub fn parse_parallel_with(
    input: &[u8],
    options: ParseOptions,
    threads: usize,
) -> Result<ParseOutcome, StepError> {
    if threads <= 1 || !crate::is_step_file(input) {
        return parse_with(input, options);
    }
    try_parallel(input, options, threads).map_or_else(|| parse_with(input, options), Ok)
}

/// The parallel attempt; `None` means "use the sequential result".
fn try_parallel(input: &[u8], options: ParseOptions, threads: usize) -> Option<ParseOutcome> {
    let (header, data_start) = parse_prefix(input, options).ok()??;
    let mut diagnostics = Vec::new();
    let bounds = boundaries(input, data_start, threads);
    let starts = std::iter::once(data_start).chain(bounds.iter().copied());
    let ends = bounds
        .iter()
        .copied()
        .map(Some)
        .chain(std::iter::once(None));
    let slices: Vec<(usize, Option<usize>)> = starts.zip(ends).collect();

    let chunks: Vec<Result<Chunk, StepError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = slices
            .iter()
            .map(|&(start, end)| scope.spawn(move || parse_chunk(input, options, start, end)))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });

    let mut parsed = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let chunk = chunk.ok()?;
        if !chunk.aligned {
            return None;
        }
        parsed.push(chunk);
    }

    let mut references = options.check_references.then(ReferenceCheck::default);
    let mut records = Vec::with_capacity(parsed.iter().map(|chunk| chunk.records.len()).sum());
    for chunk in parsed {
        // Slices are in source order, and so are the diagnostics within
        // each, so plain concatenation is the sequential order. With the
        // reference check on, `finish` re-sorts everything by position
        // (stably) exactly as the sequential parse does.
        diagnostics.extend(chunk.diagnostics);
        for (record, span) in chunk.records.into_iter().zip(chunk.spans) {
            if let Some(check) = &mut references {
                check.record(&record, span, &mut diagnostics);
            }
            records.push(record);
        }
    }
    if let Some(check) = references {
        check.finish(&mut diagnostics);
    }

    Some(ParseOutcome {
        exchange: Exchange {
            header,
            data: DataSection { records },
        },
        diagnostics,
    })
}

/// Up to `threads - 1` strictly increasing split offsets after `data_start`,
/// each just past a `;` that is followed by optional whitespace and `#`
/// plus a digit. A guess only: the slice parsers verify every one.
fn boundaries(input: &[u8], data_start: usize, threads: usize) -> Vec<usize> {
    let step = (input.len() - data_start) / threads;
    let mut out = Vec::with_capacity(threads - 1);
    let mut from = data_start;
    for k in 1..threads {
        let target = data_start + step * k;
        if target <= from {
            continue;
        }
        match next_boundary(input, target) {
            Some(boundary) => {
                out.push(boundary);
                from = boundary;
            }
            None => break,
        }
    }
    out
}

fn next_boundary(input: &[u8], from: usize) -> Option<usize> {
    let mut position = from;
    while let Some(offset) = memchr::memchr(b';', &input[position..]) {
        let after = position + offset + 1;
        let rest = &input[after..];
        let blank = rest
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        if rest.get(blank) == Some(&b'#') && rest.get(blank + 1).is_some_and(u8::is_ascii_digit) {
            return Some(after);
        }
        position = after;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{boundaries, next_boundary, try_parallel};
    use crate::parser::parse_chunk;
    use crate::{parse_with, ParseOptions};
    use std::fmt::Write as _;

    /// Records with no literal that looks like a boundary, so every split
    /// guess is a real boundary and the parallel path must be taken.
    fn plain(n: usize) -> Vec<u8> {
        let mut out = String::from(
            "ISO-10303-21;HEADER;FILE_DESCRIPTION(('d'),'2;1');\
FILE_NAME('n','t',('a'),('o'),'p','s','z');FILE_SCHEMA(('IFC4'));ENDSEC;DATA;\n",
        );
        for id in 1..=n {
            writeln!(out, "#{id}=IFCPOINT(({id}.,#{}));", n + 1 - id)
                .expect("writing to a String cannot fail");
        }
        out.push_str("ENDSEC;END-ISO-10303-21;\n");
        out.into_bytes()
    }

    #[test]
    fn clean_input_takes_the_parallel_path_and_matches() {
        let input = plain(2_000);
        for options in [
            ParseOptions::strict(),
            ParseOptions::lenient().check_references(true),
        ] {
            for threads in [2, 4, 8] {
                let parallel =
                    try_parallel(&input, options, threads).expect("clean input must not fall back");
                assert_eq!(
                    Ok(parallel),
                    parse_with(&input, options),
                    "threads={threads}"
                );
            }
        }
    }

    #[test]
    fn a_slice_whose_end_is_inside_a_record_stops_there_unaligned() {
        let input = plain(2_000);
        let text = std::str::from_utf8(&input).expect("ascii");
        let start = text.find("#1=").expect("first record");
        let middle = text.find("#1000=").expect("record 1000") + 3;
        let chunk = parse_chunk(&input, ParseOptions::strict(), start, Some(middle))
            .expect("parses up to the overshoot");
        assert!(!chunk.aligned);
        // It must stop at the first record past `end`, not run to the end
        // of the file: that is wasted work on every misaligned split.
        assert_eq!(chunk.records.len(), 1_000);
    }

    #[test]
    fn a_boundary_is_just_past_a_semicolon_before_an_id() {
        let input: &[u8] = b"DATA;#1=A();\n  #2=B('x;y');#3=C();";
        let after = |needle: &[u8]| {
            input
                .windows(needle.len())
                .position(|window| window == needle)
                .expect("needle present")
                + needle.len()
        };
        assert_eq!(next_boundary(input, 0), Some(after(b"DATA;")));
        assert_eq!(next_boundary(input, 6), Some(after(b"#1=A();")));
        // `;y` inside the string is not followed by `#digit`, so the next
        // boundary after #2 starts is the end of #2.
        assert_eq!(next_boundary(input, 16), Some(after(b"'x;y');")));
        assert_eq!(next_boundary(input, input.len() - 3), None);
    }

    #[test]
    fn boundaries_are_strictly_increasing_and_bounded() {
        let mut input = b"DATA;".to_vec();
        for id in 1..=100 {
            input.extend_from_slice(format!("#{id}=A({id});").as_bytes());
        }
        for threads in [2, 3, 8, 64, 500] {
            let cuts = boundaries(&input, 5, threads);
            assert!(cuts.len() < threads);
            assert!(cuts.windows(2).all(|pair| pair[0] < pair[1]));
            assert!(cuts.iter().all(|&cut| cut > 5 && cut < input.len()));
            assert!(cuts.iter().all(|&cut| input[cut - 1] == b';'));
        }
    }
}
