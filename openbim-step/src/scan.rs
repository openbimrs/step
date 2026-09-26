//! Lazy record index: frame every data record without decoding it.
//!
//! [`scan`] reads the header strictly and then walks the data section record
//! by record, yielding each record's instance id, record name and byte span.
//! Parameters are not tokenized; [`decode_record`] parses one record from its
//! span when it is needed. This is the shape of a lazy model -- index once,
//! decode what is touched -- and it is several times faster than a full
//! parse when most records are never decoded.
//!
//! Correctness rests on the parser, not on the framing. Between records the
//! parser's whole state is its byte offset, so a record decodes exactly as a
//! whole-file parse reads it, and [`decode_record`] fails unless the record
//! ends exactly at the end of its span. Everything between records -- trivia,
//! `ENDSEC;` and the end marker -- is read with the lexer, just as the parser
//! reads it. Hence: if the scan and the decode of every record succeed, then
//! [`parse`](crate::parse) succeeds and returns exactly those records. A
//! framing mistake can only surface as an error, never as a different record.
//!
//! What the scan does not do is check the syntax inside a record: that
//! happens when the record is decoded. A file with a malformed parameter
//! scans cleanly and fails on decoding that record.

use crate::lexer::{Lexer, Token};
use crate::parser::{decode_borrowed, decode_owned, parse_prefix};
use crate::{DataRecord, HeaderSection, InstanceId, ParseOptions, Span, StepError};
use std::borrow::Cow;

/// A data section indexed by [`scan`]: the parsed header and an iterator
/// over the framed data records.
#[derive(Debug, Clone)]
pub struct Scan<'a> {
    input: &'a [u8],
    header: HeaderSection,
    data_start: usize,
}

/// One framed data record: `#id = NAME(...);` or `#id = (A(...)B(...));`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedRecord<'a> {
    /// Instance id.
    pub id: InstanceId,
    /// Record name as written, for a simple instance; `None` for a complex
    /// instance, whose names are only known after decoding. Compare with
    /// `eq_ignore_ascii_case`: the owned parse upper-cases names.
    pub name: Option<Cow<'a, str>>,
    /// Bytes from `#` through the terminating `;`.
    pub span: Span,
}

/// Indexes the data records of a physical file without decoding them.
///
/// The header is parsed and checked exactly as [`parse`](crate::parse) does.
/// # Errors
///
/// Returns [`StepError`] for the wrong physical-file marker and for any
/// header or section-structure defect up to `DATA;`. Defects after that are
/// reported by the [`Scan::records`] iterator.
pub fn scan(input: &[u8]) -> Result<Scan<'_>, StepError> {
    if !crate::is_step_file(input) {
        return Err(StepError::not_step("missing ISO-10303-21 marker"));
    }
    match parse_prefix(input, ParseOptions::strict())? {
        Some((header, data_start)) => Ok(Scan {
            input,
            header,
            data_start,
        }),
        // No `DATA;`: a full parse reports why the file is incomplete.
        None => Err(crate::parse(input).err().unwrap_or_else(|| {
            StepError::syntax(Span::new(input.len(), input.len()), "missing DATA section")
        })),
    }
}

impl<'a> Scan<'a> {
    /// The parsed header section.
    #[must_use]
    pub const fn header(&self) -> &HeaderSection {
        &self.header
    }

    /// The input this scan indexes.
    #[must_use]
    pub const fn input(&self) -> &'a [u8] {
        self.input
    }

    /// Iterates over the data records in source order. After the last
    /// record the iterator checks `ENDSEC;` and the end marker; a defect
    /// anywhere is yielded as an error, after which the iterator ends.
    #[must_use]
    pub fn records(&self) -> Records<'a> {
        let mut lexer = Lexer::new(self.input);
        lexer.resume_at(self.data_start);
        Records {
            input: self.input,
            lexer,
            done: false,
        }
    }

    /// Decodes one scanned record with owned text, exactly as
    /// [`parse`](crate::parse) returns it.
    /// # Errors
    ///
    /// Returns the syntax error inside the record, if any.
    pub fn decode(&self, record: &ScannedRecord<'_>) -> Result<DataRecord, StepError> {
        decode_record(self.input, record.span)
    }
}

/// Parses the data record that occupies exactly `span` of `input`, with
/// owned text as [`parse`](crate::parse) returns it.
///
/// `span` should come from [`scan`]. Any span is checked to hold exactly one
/// record, but only a scanned span is known to be a record of the data
/// section rather than, say, text inside a string literal.
/// # Errors
///
/// Returns a syntax error for a malformed record, a span that does not start
/// at `#`, or a record that does not end exactly at `span.end`.
pub fn decode_record(input: &[u8], span: Span) -> Result<DataRecord, StepError> {
    decode_owned(input, span)
}

/// [`decode_record`] with text borrowed from `input` wherever it needs no
/// rewriting, as [`parse_events_borrowed`](crate::parse_events_borrowed)
/// returns it.
/// # Errors
///
/// The same errors as [`decode_record`].
pub fn decode_record_borrowed(
    input: &[u8],
    span: Span,
) -> Result<DataRecord<Cow<'_, str>>, StepError> {
    decode_borrowed(input, span)
}

/// Iterator over framed data records; see [`Scan::records`].
#[derive(Debug, Clone)]
pub struct Records<'a> {
    input: &'a [u8],
    lexer: Lexer<'a>,
    done: bool,
}

impl<'a> Iterator for Records<'a> {
    type Item = Result<ScannedRecord<'a>, StepError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let step = self.step();
        if !matches!(step, Ok(Some(_))) {
            self.done = true;
        }
        step.transpose()
    }
}

impl<'a> Records<'a> {
    /// Frames the next record, or checks the end of the file after the last.
    fn step(&mut self) -> Result<Option<ScannedRecord<'a>>, StepError> {
        if let Some(head) = self.lexer.record_head() {
            let end = self.record_end(head.start)?;
            return Ok(Some(ScannedRecord {
                id: InstanceId::from_digits(head.id),
                name: head.name.map(ascii_borrowed),
                span: Span::new(head.start, end),
            }));
        }
        let token = self.token("unterminated section (expected ENDSEC)")?;
        match token.value {
            Token::Id(id) => {
                let start = token.span.start;
                let equals = self.token("expected '=' after instance id")?;
                if equals.value != Token::Equals {
                    return Err(StepError::syntax(
                        equals.span,
                        "expected '=' after instance id",
                    ));
                }
                let body = self.token("missing record body")?;
                let name = match body.value {
                    Token::Name(name) => Some(ascii(name)),
                    Token::OpenParen => None,
                    _ => {
                        return Err(StepError::syntax(
                            body.span,
                            "expected record name or complex instance",
                        ));
                    }
                };
                let end = self.record_end(start)?;
                Ok(Some(ScannedRecord {
                    id: InstanceId::from_ascii_digits(&id).expect("lexer validates instance ids"),
                    name,
                    span: Span::new(start, end),
                }))
            }
            Token::Name(name) if name.eq_ignore_ascii_case(b"ENDSEC") => {
                self.expect(&Token::Semicolon, "expected ';' after ENDSEC")?;
                let marker =
                    self.token("physical file requires start, HEADER, DATA, and end markers")?;
                if !matches!(&marker.value, Token::Name(name) if name.eq_ignore_ascii_case(b"END-ISO-10303-21"))
                {
                    return Err(StepError::syntax(
                        marker.span,
                        format!("unexpected token {:?} in BeforeEnd", marker.value),
                    ));
                }
                self.expect(&Token::Semicolon, "expected ';' after END-ISO-10303-21")?;
                if let Some(extra) = self.lexer.next_spanned()? {
                    return Err(StepError::syntax(
                        extra.span,
                        "content after END-ISO-10303-21",
                    ));
                }
                Ok(None)
            }
            value => Err(StepError::syntax(
                token.span,
                format!("unexpected token {value:?} in Data"),
            )),
        }
    }

    fn token(&mut self, at_end: &str) -> Result<crate::Spanned<Token<'a>>, StepError> {
        self.lexer.next_spanned()?.ok_or_else(|| {
            let end = self.input.len();
            StepError::syntax(Span::new(end, end), at_end)
        })
    }

    fn expect(&mut self, expected: &Token<'_>, message: &str) -> Result<(), StepError> {
        let token = self.token(message)?;
        if &token.value == expected {
            Ok(())
        } else {
            Err(StepError::syntax(token.span, message))
        }
    }

    /// Finds the `;` that ends the record starting at `start`, from the
    /// lexer's current offset, and leaves the lexer just past it.
    ///
    /// Outside literals only `;`, `'` and `/` can matter, so the scan jumps
    /// between them. A string is skipped by the lexer's own string reader. A
    /// `/` is skipped as a comment when the lexer reads one there, else as a
    /// single byte: a lone `/` is a syntax error the decode reports. Binary
    /// literals need no case of their own: a valid body is hex digits, and
    /// an invalid one fails to decode.
    fn record_end(&mut self, start: usize) -> Result<usize, StepError> {
        let input = self.input;
        let mut position = self.lexer.offset();
        loop {
            let Some(offset) = memchr::memchr3(b';', b'\'', b'/', &input[position..]) else {
                return Err(StepError::syntax(
                    Span::new(start, input.len()),
                    "unterminated data record",
                ));
            };
            position += offset;
            match input[position] {
                b';' => {
                    position += 1;
                    self.lexer.resume_at(position);
                    return Ok(position);
                }
                b'\'' => {
                    self.lexer.skip_text(position)?;
                    position = self.lexer.offset();
                }
                _ => {
                    self.lexer.resume_at(position);
                    self.lexer.skip_trivia()?;
                    position = self.lexer.offset().max(position + 1);
                }
            }
        }
    }
}

/// A name token as text. The lexer admits only ASCII in names, so the lossy
/// conversion never replaces anything.
fn ascii(bytes: Cow<'_, [u8]>) -> Cow<'_, str> {
    match bytes {
        Cow::Borrowed(bytes) => ascii_borrowed(bytes),
        Cow::Owned(bytes) => Cow::Owned(String::from_utf8_lossy(&bytes).into_owned()),
    }
}

fn ascii_borrowed(bytes: &[u8]) -> Cow<'_, str> {
    std::str::from_utf8(bytes).map_or_else(|_| String::from_utf8_lossy(bytes), Cow::Borrowed)
}
