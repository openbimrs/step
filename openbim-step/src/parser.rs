//! Semantic parser and event/sink interface.
//!
//! Parsing is structural: record names and parameters are retained without a
//! domain schema. Unknown header and data records therefore survive a
//! parse/write/reparse cycle.

use std::borrow::Cow;

use crate::escape;
use crate::lexer::{Lexer, Token};
use crate::recovery::{Diagnostic, OnMalformed, ParseOptions, ParseOutcome};
use crate::references::ReferenceCheck;
use crate::{
    DataRecord, DataSection, Exchange, HeaderRecord, HeaderSection, InstanceId, Parameter, Record,
    Span, Spanned, StepError,
};

/// A semantic parser event.
#[derive(Debug, Clone, PartialEq)]
pub enum Event<S = String> {
    /// Entered `HEADER;`.
    StartHeader,
    /// Parsed one header record.
    HeaderRecord(HeaderRecord<S>),
    /// Reached the header `ENDSEC;`.
    EndHeader,
    /// Entered `DATA;`.
    StartData,
    /// Parsed one data record.
    DataRecord(DataRecord<S>),
    /// Reached the data `ENDSEC;`.
    EndData,
}

/// Consumer for parse events.
pub trait EventSink<S = String> {
    /// Receives one event. Events are delivered in source order.
    fn event(&mut self, event: Event<S>);
}

impl<S, F> EventSink<S> for F
where
    F: FnMut(Event<S>),
{
    fn event(&mut self, event: Event<S>) {
        self(event);
    }
}

/// Parses a physical file into an owned generic exchange structure.
/// # Errors
///
/// Returns [`StepError`] for the wrong
/// physical-file marker, or a spanned lexical/syntax diagnostic otherwise.
pub fn parse(input: &[u8]) -> Result<Exchange, StepError> {
    parse_with(input, ParseOptions::strict()).map(|outcome| outcome.exchange)
}

/// Parses a physical file under explicit [`ParseOptions`].
///
/// With [`OnMalformed::Skip`] an unparsable data record is reported as a
/// [`Diagnostic`] and the parser resynchronizes on the next record or section
/// end, so a consumer can load a damaged file and still show exactly what was
/// dropped. Header structure stays strict under every policy: mandatory header
/// records are file-level invariants, not recoverable payload.
/// # Errors
///
/// Returns [`StepError`] for the wrong physical-file marker, for any header or
/// section-structure defect, and for data defects when the policy is
/// [`OnMalformed::Abort`].
pub fn parse_with(input: &[u8], options: ParseOptions) -> Result<ParseOutcome, StepError> {
    #[derive(Default)]
    struct Builder {
        header: HeaderSection,
        data: DataSection,
    }

    impl EventSink for Builder {
        fn event(&mut self, event: Event) {
            match event {
                Event::HeaderRecord(record) => self.header.records.push(record),
                Event::DataRecord(record) => self.data.records.push(record),
                Event::StartHeader | Event::EndHeader | Event::StartData | Event::EndData => {}
            }
        }
    }

    let mut builder = Builder::default();
    let diagnostics = parse_events_with(input, &mut builder, options)?;
    // The record array grew by doubling; give the unused tail back once.
    // A reallocation of the final size, instead of up to half again as
    // much memory for as long as the exchange lives.
    builder.data.records.shrink_to_fit();
    Ok(ParseOutcome {
        exchange: Exchange {
            header: builder.header,
            data: builder.data,
        },
        diagnostics,
    })
}

/// Parses a physical file and sends semantic records to `sink`.
///
/// Unlike [`parse`], this API does not accumulate an [`Exchange`]. It is useful
/// for import pipelines that index, validate, or transform records as they are
/// read.
/// # Errors
///
/// Returns the same physical-file and syntax diagnostics as [`parse`].
pub fn parse_events(input: &[u8], sink: &mut impl EventSink) -> Result<(), StepError> {
    parse_events_with(input, sink, ParseOptions::strict()).map(|_| ())
}

/// Streams semantic records under explicit [`ParseOptions`], returning the
/// non-fatal diagnostics collected on the way.
/// # Errors
///
/// Returns the same physical-file, header, and structure diagnostics as
/// [`parse_with`].
pub fn parse_events_with(
    input: &[u8],
    sink: &mut impl EventSink,
    options: ParseOptions,
) -> Result<Vec<Diagnostic>, StepError> {
    if !crate::is_step_file(input) {
        return Err(StepError::not_step("missing ISO-10303-21 marker"));
    }
    let mut parser = Parser::new(input);
    parser.options = options;
    parser.references = options.check_references.then(ReferenceCheck::default);
    parser.parse(sink)?;
    Ok(parser.diagnostics)
}

/// Streams semantic records whose text borrows from `input` where it can.
///
/// Identical to [`parse_events_with`] -- same events, order, diagnostics and
/// errors -- except that names, numbers, enumerations and binaries are
/// `Cow::Borrowed` slices of `input` unless the source interrupted them with
/// an ignored control (TAB, LF, CR, FF), and strings are borrowed unless they
/// contain an escape or a quote. Name case is preserved as written (the owned
/// API upper-cases); compare with `eq_ignore_ascii_case`. A consumer that
/// converts every value anyway skips one allocation per value this way.
/// # Errors
///
/// Returns the same diagnostics as [`parse_events_with`].
pub fn parse_events_borrowed<'a>(
    input: &'a [u8],
    sink: &mut impl EventSink<Cow<'a, str>>,
    options: ParseOptions,
) -> Result<Vec<Diagnostic>, StepError> {
    if !crate::is_step_file(input) {
        return Err(StepError::not_step("missing ISO-10303-21 marker"));
    }
    let mut parser = Parser::new(input);
    parser.options = options;
    parser.references = options.check_references.then(ReferenceCheck::default);
    parser.parse(sink)?;
    Ok(parser.diagnostics)
}

/// How the parser turns lexemes into the caller's string type.
///
/// `String` reproduces the owned API exactly (names upper-cased, lossy UTF-8).
/// `Cow<'a, str>` borrows from the input whenever the bytes are valid UTF-8
/// and need no rewriting, and keeps name case as written.
trait Text<'a>: Sized {
    /// A record, typed-parameter or enumeration name.
    fn name(bytes: Cow<'a, [u8]>) -> Self;
    /// A number or binary lexeme.
    fn lexeme(bytes: Cow<'a, [u8]>) -> Self;
    /// An escaped string body.
    fn text(raw: Cow<'a, [u8]>) -> Self;
}

impl<'a> Text<'a> for String {
    fn name(bytes: Cow<'a, [u8]>) -> Self {
        upper(&bytes)
    }

    fn lexeme(bytes: Cow<'a, [u8]>) -> Self {
        lexeme_string(bytes)
    }

    fn text(raw: Cow<'a, [u8]>) -> Self {
        escape::decode(&raw)
    }
}

impl<'a> Text<'a> for Cow<'a, str> {
    fn name(bytes: Cow<'a, [u8]>) -> Self {
        borrowed_str(bytes)
    }

    fn lexeme(bytes: Cow<'a, [u8]>) -> Self {
        borrowed_str(bytes)
    }

    fn text(raw: Cow<'a, [u8]>) -> Self {
        // Without a quote or backslash there is nothing to decode, so the
        // decoded text is the raw body itself.
        match raw {
            Cow::Borrowed(bytes) if memchr::memchr2(b'\\', b'\'', bytes).is_none() => {
                borrowed_str(Cow::Borrowed(bytes))
            }
            raw => Cow::Owned(escape::decode(&raw)),
        }
    }
}

/// Borrows valid UTF-8 as `str`; otherwise the same lossy conversion as the
/// owned API.
fn borrowed_str(bytes: Cow<'_, [u8]>) -> Cow<'_, str> {
    match bytes {
        // `from_utf8` first: names, numbers and ids are ASCII by
        // construction, and its validation is much cheaper than the chunked
        // walk `from_utf8_lossy` does before it can return a borrow. The
        // result is identical: lossy conversion of valid UTF-8 borrows it.
        Cow::Borrowed(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => Cow::Borrowed(text),
            Err(_) => String::from_utf8_lossy(bytes),
        },
        Cow::Owned(bytes) => Cow::Owned(lexeme_string(Cow::Owned(bytes))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    BeforeStart,
    BeforeHeader,
    Header,
    BeforeData,
    Data,
    BeforeEnd,
    Done,
}

struct Parser<'a, S> {
    input: &'a [u8],
    lexer: Lexer<'a>,
    lookahead: Option<Spanned<Token<'a>>>,
    last_end: usize,
    phase: Phase,
    header_records_seen: usize,
    options: ParseOptions,
    diagnostics: Vec<Diagnostic>,
    /// Present only when the caller opted into reference checking.
    references: Option<ReferenceCheck>,
    /// Where to stop early; only the parallel driver sets anything else.
    stop: Stop,
    /// The offset the parse stopped at, when it stopped as `stop` asked.
    stopped: Option<usize>,
    /// Source span of every data record emitted, when collected.
    record_spans: Option<Vec<Span>>,
    /// Parameters of the lists being parsed, innermost last. Each list
    /// pushes onto it from its own base and drains its tail into a `Vec`
    /// of exactly its length, so no list carries growth slack or pays for
    /// regrowth, and the stack's own buffer is reused for the whole parse.
    scratch: Vec<Parameter<S>>,
}

/// Where a restricted parse ends. The parallel driver parses the header up
/// to `DATA;` and then each slice of the data section on its own thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    /// Parse the whole input.
    Never,
    /// Stop right after `DATA;`, before the first data record.
    AfterDataStart,
    /// Stop when a record ends exactly at this offset.
    AtOffset(usize),
}

/// One slice of the data section, parsed on its own.
pub(crate) struct Chunk {
    pub(crate) records: Vec<DataRecord>,
    /// `records[i]` spans `spans[i]`, as the reference check needs.
    pub(crate) spans: Vec<Span>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Whether the slice ended exactly at the requested offset. `false`
    /// means the split point was not a record boundary for the parser.
    pub(crate) aligned: bool,
}

/// Parses the header through `DATA;`. Returns the header records and the
/// offset just past `DATA;`, or `None` when the parse never reaches a data
/// section. There are no diagnostics to return: header and structure
/// defects are fatal under every policy, and recovery and the reference
/// check only act inside `DATA`.
pub(crate) fn parse_prefix(
    input: &[u8],
    options: ParseOptions,
) -> Result<Option<(HeaderSection, usize)>, StepError> {
    let mut header = HeaderSection::default();
    let mut parser = Parser::new(input);
    parser.options = options;
    parser.stop = Stop::AfterDataStart;
    parser.parse(&mut |event: Event| {
        if let Event::HeaderRecord(record) = event {
            header.records.push(record);
        }
    })?;
    debug_assert!(parser.diagnostics.is_empty());
    Ok(parser.stopped.map(|offset| (header, offset)))
}

/// Parses the data section from `start`, which must be a record boundary,
/// to `end` (`None`: to the end of the file, including `ENDSEC` and the end
/// marker). The parser state at a record boundary is fully determined by
/// the offset, so this is exactly what a whole-file parse does there.
pub(crate) fn parse_chunk(
    input: &[u8],
    options: ParseOptions,
    start: usize,
    end: Option<usize>,
) -> Result<Chunk, StepError> {
    let mut records = Vec::new();
    let mut parser = Parser::new(input);
    parser.options = options;
    parser.phase = Phase::Data;
    parser.lexer.resume_at(start);
    parser.last_end = start;
    parser.record_spans = Some(Vec::new());
    parser.stop = end.map_or(Stop::Never, Stop::AtOffset);
    parser.parse(&mut |event: Event| {
        if let Event::DataRecord(record) = event {
            records.push(record);
        }
    })?;
    Ok(Chunk {
        records,
        spans: parser.record_spans.take().unwrap_or_default(),
        diagnostics: parser.diagnostics,
        aligned: end.is_none() || parser.stopped.is_some(),
    })
}

/// Scratch capacity for decoding a single record: records of real files
/// rarely hold more than this many parameters across their open lists.
const RECORD_SCRATCH: usize = 32;

/// Parses the one data record that occupies exactly `span`. Between records
/// the parser's whole state is its offset, so this is what a whole-file parse
/// produces for that record, and a record that does not end exactly at
/// `span.end` is an error rather than a different record.
fn parse_record_at<'a, S: Text<'a>>(
    input: &'a [u8],
    span: Span,
) -> Result<DataRecord<S>, StepError> {
    if span.start > span.end || span.end > input.len() {
        return Err(StepError::invalid_argument(format!(
            "record span {}..{} is outside the {}-byte input",
            span.start,
            span.end,
            input.len()
        )));
    }
    let mut parser = Parser::new(input);
    // One allocation that fits the parameters of almost every record,
    // instead of regrowing the stack from empty for each decoded record.
    parser.scratch = Vec::with_capacity(RECORD_SCRATCH);
    parser.phase = Phase::Data;
    parser.lexer.resume_at(span.start);
    parser.last_end = span.start;
    let token = parser
        .next()?
        .ok_or_else(|| StepError::syntax(span, "expected a data record"))?;
    let Token::Id(id) = token.value else {
        return Err(StepError::syntax(token.span, "expected a data record"));
    };
    let record = parser.parse_data_record(&id, token.span)?;
    if parser.lexer.offset() != span.end {
        return Err(StepError::syntax(
            Span::new(span.start, parser.lexer.offset()),
            "data record does not end at the end of its span",
        ));
    }
    Ok(record)
}

/// [`parse_record_at`] with owned text, as [`parse`] returns it.
pub(crate) fn decode_owned(input: &[u8], span: Span) -> Result<DataRecord, StepError> {
    parse_record_at(input, span)
}

/// [`parse_record_at`] with text borrowed where it can be, as
/// [`parse_events_borrowed`] returns it.
pub(crate) fn decode_borrowed(
    input: &[u8],
    span: Span,
) -> Result<DataRecord<Cow<'_, str>>, StepError> {
    parse_record_at(input, span)
}

impl<'a, S: Text<'a>> Parser<'a, S> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            lexer: Lexer::new(input),
            lookahead: None,
            last_end: 0,
            phase: Phase::BeforeStart,
            header_records_seen: 0,
            options: ParseOptions::strict(),
            diagnostics: Vec::new(),
            references: None,
            stop: Stop::Never,
            stopped: None,
            record_spans: None,
            scratch: Vec::new(),
        }
    }

    // Keeping section dispatch together makes the state-machine transitions auditable.
    #[allow(clippy::too_many_lines)]
    fn parse(&mut self, sink: &mut impl EventSink<S>) -> Result<(), StepError> {
        loop {
            if let Stop::AtOffset(end) = self.stop {
                // Between records there is no lookahead, and the lexer sits
                // just past the last `;`. Landing exactly on `end` is the
                // only success; passing it means the split point was inside
                // a record or literal, and the slice is not usable.
                if self.lookahead.is_none() {
                    let offset = self.lexer.offset();
                    if offset == end {
                        self.stopped = Some(end);
                        return Ok(());
                    }
                    if offset > end {
                        return Ok(());
                    }
                }
            }
            let token = match self.next() {
                Ok(Some(token)) => token,
                Ok(None) => break,
                Err(error) => {
                    // A lexical defect inside DATA is recoverable: the damaged
                    // bytes belong to one record, not to the file structure.
                    self.recover_or_fail(error, None)?;
                    continue;
                }
            };
            if self.phase == Phase::Done {
                return Err(StepError::syntax(
                    token.span,
                    "content after END-ISO-10303-21",
                ));
            }
            match token.value {
                Token::Name(name) if name.eq_ignore_ascii_case(b"ISO-10303-21") => {
                    if self.phase != Phase::BeforeStart {
                        return Err(StepError::syntax(
                            token.span,
                            "unexpected ISO-10303-21 marker",
                        ));
                    }
                    self.expect_semicolon("after ISO-10303-21")?;
                    self.phase = Phase::BeforeHeader;
                }
                Token::Name(name) if name.eq_ignore_ascii_case(b"HEADER") => {
                    if self.phase != Phase::BeforeHeader {
                        return Err(StepError::syntax(token.span, "unexpected HEADER section"));
                    }
                    self.expect_semicolon("after HEADER")?;
                    self.phase = Phase::Header;
                    sink.event(Event::StartHeader);
                }
                Token::Name(name) if name.eq_ignore_ascii_case(b"DATA") => {
                    if self.phase != Phase::BeforeData {
                        return Err(StepError::syntax(token.span, "unexpected DATA section"));
                    }
                    self.expect_semicolon("after DATA")?;
                    self.phase = Phase::Data;
                    sink.event(Event::StartData);
                    if self.stop == Stop::AfterDataStart {
                        self.stopped = Some(self.lexer.offset());
                        return Ok(());
                    }
                }
                Token::Name(name) if name.eq_ignore_ascii_case(b"ENDSEC") => {
                    self.expect_semicolon("after ENDSEC")?;
                    match self.phase {
                        Phase::Header => {
                            if self.header_records_seen < 3 {
                                return Err(StepError::syntax(
                                    token.span,
                                    "missing mandatory STEP header record",
                                ));
                            }
                            sink.event(Event::EndHeader);
                            self.phase = Phase::BeforeData;
                        }
                        Phase::Data => {
                            if let Some(check) = self.references.take() {
                                check.finish(&mut self.diagnostics);
                            }
                            sink.event(Event::EndData);
                            self.phase = Phase::BeforeEnd;
                        }
                        _ => {
                            return Err(StepError::syntax(token.span, "ENDSEC outside a section"));
                        }
                    }
                }
                Token::Name(name) if name.eq_ignore_ascii_case(b"END-ISO-10303-21") => {
                    if self.phase != Phase::BeforeEnd {
                        return Err(StepError::syntax(
                            token.span,
                            "unexpected END-ISO-10303-21 marker",
                        ));
                    }
                    self.expect_semicolon("after END-ISO-10303-21")?;
                    self.phase = Phase::Done;
                }
                Token::Name(name) if self.phase == Phase::Header => {
                    self.validate_header_record(&name, token.span)?;
                    let parameters = self.parse_arguments()?;
                    self.expect_semicolon("after header record")?;
                    sink.event(Event::HeaderRecord(HeaderRecord {
                        name: S::name(name),
                        parameters,
                    }));
                }
                Token::Id(id) if self.phase == Phase::Data => {
                    let start = token.span.start;
                    match self.parse_data_record(&id, token.span) {
                        Ok(record) => {
                            if let Some(check) = &mut self.references {
                                let span = Span::new(start, self.last_end);
                                check.record(&record, span, &mut self.diagnostics);
                            }
                            if let Some(spans) = &mut self.record_spans {
                                spans.push(Span::new(start, self.last_end));
                            }
                            sink.event(Event::DataRecord(record));
                        }
                        Err(error) => self.recover_or_fail(error, Some(start))?,
                    }
                }
                _ => {
                    let error = StepError::syntax(
                        token.span,
                        format!("unexpected token {:?} in {:?}", token.value, self.phase),
                    );
                    self.recover_or_fail(error, Some(token.span.start))?;
                }
            }
        }
        if self.phase != Phase::Done {
            let detail = if matches!(self.phase, Phase::Header | Phase::Data) {
                "unterminated section (expected ENDSEC)"
            } else {
                "physical file requires start, HEADER, DATA, and end markers"
            };
            return Err(StepError::syntax(self.eof_span(), detail));
        }
        Ok(())
    }

    /// Parses one `#id = ...;` data record, assuming the id token was consumed.
    fn parse_data_record(&mut self, id: &[u8], id_span: Span) -> Result<DataRecord<S>, StepError> {
        self.expect_equals()?;
        let record_token = self.next()?.ok_or_else(|| {
            StepError::syntax(Span::new(id_span.end, id_span.end), "missing record body")
        })?;
        let records = match record_token.value {
            Token::Name(name) => vec![self.parse_named_record(name)?],
            Token::OpenParen => {
                let mut records = Vec::new();
                loop {
                    if !self
                        .peek()?
                        .is_some_and(|next| next.value != Token::CloseParen)
                    {
                        break;
                    }
                    let component = self.next()?.ok_or_else(|| {
                        StepError::syntax(self.eof_span(), "missing complex record")
                    })?;
                    let Token::Name(name) = component.value else {
                        return Err(StepError::syntax(
                            component.span,
                            "expected complex record name",
                        ));
                    };
                    records.push(self.parse_named_record(name)?);
                }
                let close = self.next()?.ok_or_else(|| {
                    StepError::syntax(self.eof_span(), "unterminated complex instance")
                })?;
                if close.value != Token::CloseParen {
                    return Err(StepError::syntax(
                        close.span,
                        "expected ')' after complex instance",
                    ));
                }
                if records.is_empty() {
                    return Err(StepError::syntax(
                        record_token.span,
                        "complex instance must contain a record",
                    ));
                }
                records
            }
            _ => {
                return Err(StepError::syntax(
                    record_token.span,
                    "expected record name or complex instance",
                ));
            }
        };
        self.expect_semicolon("after data record")?;
        Ok(DataRecord {
            id: InstanceId::from_ascii_digits(id).expect("lexer validates instance ids"),
            records,
        })
    }

    /// Applies the malformed-record policy.
    ///
    /// Recovery is deliberately narrow. It applies only inside `DATA`, only
    /// when the caller opted in, and it always advances the cursor, so a
    /// damaged file cannot loop. Header and section-structure defects stay
    /// fatal under every policy: they describe the file, not one payload
    /// record, and silently continuing past them would produce a model whose
    /// provenance is unknown.
    fn recover_or_fail(
        &mut self,
        error: StepError,
        record_start: Option<usize>,
    ) -> Result<(), StepError> {
        if self.options.on_malformed_record != OnMalformed::Skip || self.phase != Phase::Data {
            return Err(error);
        }
        let start = record_start.unwrap_or_else(|| error.span().start);
        // Resynchronize from just past the record's first byte, NOT from the
        // end of the error span. A diagnostic can legitimately span the token
        // that follows the damage -- including `ENDSEC` -- and resuming past
        // it would swallow the section terminator.
        let resume = self.resync_from(start.saturating_add(1));
        self.lookahead = None;
        self.lexer.resume_at(resume);
        self.last_end = resume;
        self.diagnostics.push(Diagnostic::skipped_record(
            Span::new(start, resume),
            format!("skipped malformed data record: {}", error.detail()),
        ));
        Ok(())
    }

    /// Finds the next byte offset at which parsing can safely restart.
    ///
    /// Scans raw bytes rather than tokens because the tokenizer is what
    /// failed. All three literal kinds -- quoted strings, binary literals, and
    /// comments -- are tracked, so a `;` or `ENDSEC` inside a literal is not
    /// mistaken for a record boundary. Scanning a literal's payload as code
    /// would let recovery fabricate records that were never in the source.
    /// A section terminator stops the scan *before* it is consumed, so
    /// recovery can never swallow the end of `DATA`.
    fn resync_from(&self, from: usize) -> usize {
        // Which literal the scanner is currently inside. STEP has three, and
        // all of them can contain bytes that look like record syntax.
        enum Literal {
            None,
            // `'...'`, where `''` is an escaped apostrophe rather than a close.
            Text,
            // `"...."`, with no doubling rule: the first `"` closes it.
            Binary,
        }

        let mut position = from.min(self.input.len());
        let mut literal = Literal::None;
        while position < self.input.len() {
            let byte = self.input[position];
            match literal {
                Literal::Text => {
                    // Mirror `Lexer::lex_text`: `\\` is one escaped
                    // backslash, and `\S\` escapes the next byte even when it
                    // is an apostrophe. Upper case only, as in the lexer.
                    if self.input[position..].starts_with(br"\\") {
                        position += 2;
                        continue;
                    }
                    if self.input[position..].starts_with(br"\S\") {
                        position = (position + 4).min(self.input.len());
                        continue;
                    }
                    if byte == b'\'' {
                        if self.input.get(position + 1) == Some(&b'\'') {
                            position += 2;
                            continue;
                        }
                        literal = Literal::None;
                    }
                    position += 1;
                }
                Literal::Binary => {
                    if byte == b'"' {
                        literal = Literal::None;
                    }
                    position += 1;
                }
                Literal::None => match byte {
                    b'\'' => {
                        literal = Literal::Text;
                        position += 1;
                    }
                    b'"' => {
                        literal = Literal::Binary;
                        position += 1;
                    }
                    b'/' if self.input.get(position + 1) == Some(&b'*') => {
                        position = self.input[position + 2..]
                            .windows(2)
                            .position(|window| window == b"*/")
                            .map_or(self.input.len(), |offset| position + 2 + offset + 2);
                    }
                    b';' => return position + 1,
                    _ if self.section_end_at(position) => return position,
                    _ => position += 1,
                },
            }
        }
        self.input.len()
    }

    fn section_end_at(&self, position: usize) -> bool {
        let preceded_by_word = position
            .checked_sub(1)
            .and_then(|previous| self.input.get(previous))
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
        !preceded_by_word
            && self
                .input
                .get(position..position + 6)
                .is_some_and(|bytes| bytes.eq_ignore_ascii_case(b"ENDSEC"))
    }

    fn validate_header_record(&mut self, name: &[u8], span: Span) -> Result<(), StepError> {
        const REQUIRED: [&[u8]; 3] = [b"FILE_DESCRIPTION", b"FILE_NAME", b"FILE_SCHEMA"];
        if let Some(expected) = REQUIRED.get(self.header_records_seen) {
            if !name.eq_ignore_ascii_case(expected) {
                return Err(StepError::syntax(
                    span,
                    format!(
                        "expected mandatory {} header record",
                        String::from_utf8_lossy(expected)
                    ),
                ));
            }
        } else if REQUIRED
            .iter()
            .any(|required| name.eq_ignore_ascii_case(required))
        {
            return Err(StepError::syntax(span, "duplicate mandatory header record"));
        }
        self.header_records_seen += 1;
        Ok(())
    }

    fn parse_named_record(&mut self, name: Cow<'a, [u8]>) -> Result<Record<S>, StepError> {
        Ok(Record {
            name: S::name(name),
            parameters: self.parse_arguments()?,
        })
    }

    fn parse_arguments(&mut self) -> Result<Vec<Parameter<S>>, StepError> {
        let token = self
            .next()?
            .ok_or_else(|| StepError::syntax(self.eof_span(), "expected '(' after record name"))?;
        if token.value != Token::OpenParen {
            return Err(StepError::syntax(
                token.span,
                "expected '(' after record name",
            ));
        }
        self.parse_parameter_list(0)
    }

    /// Parses a list after its `(` into a `Vec` of exactly its length.
    fn parse_parameter_list(&mut self, depth: usize) -> Result<Vec<Parameter<S>>, StepError> {
        let base = self.parse_list_onto_scratch(depth)?;
        // `Drain` knows its exact length, so this allocates once, exactly.
        Ok(self.scratch.drain(base..).collect())
    }

    /// Parses a list's parameters onto the scratch stack and returns the
    /// list's base there: the list is `scratch[base..]`. Nested lists are
    /// drained before their parent pushes again, so the stack discipline
    /// holds. On error the scratch is cut back to `base`, so a recovering
    /// parse resumes with it clean.
    fn parse_list_onto_scratch(&mut self, depth: usize) -> Result<usize, StepError> {
        let base = self.scratch.len();
        let result = self.parse_list_items(depth);
        if result.is_err() {
            self.scratch.truncate(base);
        }
        result.map(|()| base)
    }

    fn parse_list_items(&mut self, depth: usize) -> Result<(), StepError> {
        if depth > crate::MAX_PARAMETER_NESTING {
            let span = match self.peek()? {
                Some(token) => token.span,
                None => self.eof_span(),
            };
            return Err(StepError::syntax(span, "parameter nesting limit exceeded"));
        }
        if self
            .peek()?
            .is_some_and(|token| token.value == Token::CloseParen)
        {
            let _ = self.next()?;
            return Ok(());
        }
        loop {
            let parameter = self.parse_parameter(depth)?;
            self.scratch.push(parameter);
            let separator = self
                .next()?
                .ok_or_else(|| StepError::syntax(self.eof_span(), "unterminated parameter list"))?;
            match separator.value {
                Token::Comma => {}
                Token::CloseParen => return Ok(()),
                _ => {
                    return Err(StepError::syntax(
                        separator.span,
                        "expected ',' or ')' after parameter",
                    ));
                }
            }
        }
    }

    fn parse_parameter(&mut self, depth: usize) -> Result<Parameter<S>, StepError> {
        let token = self
            .next()?
            .ok_or_else(|| StepError::syntax(self.eof_span(), "expected parameter"))?;
        match token.value {
            Token::Dollar => Ok(Parameter::Null),
            Token::Star => Ok(Parameter::Derived),
            Token::Id(id) => Ok(Parameter::Ref(
                InstanceId::from_ascii_digits(&id).expect("lexer validates instance ids"),
            )),
            Token::Integer(value) => Ok(Parameter::Integer(S::lexeme(value))),
            Token::Real(value) => Ok(Parameter::Real(S::lexeme(value))),
            Token::Text(raw) => Ok(Parameter::Text(S::text(raw))),
            Token::Binary(raw) => Ok(Parameter::Binary(S::lexeme(raw))),
            Token::Keyword(keyword) if keyword.eq_ignore_ascii_case(b"T") => {
                Ok(Parameter::Bool(true))
            }
            Token::Keyword(keyword) if keyword.eq_ignore_ascii_case(b"F") => {
                Ok(Parameter::Bool(false))
            }
            Token::Keyword(keyword) if keyword.eq_ignore_ascii_case(b"U") => {
                Ok(Parameter::LogicalUnknown)
            }
            Token::Keyword(keyword) => Ok(Parameter::Enum(S::name(keyword))),
            Token::OpenParen => Ok(Parameter::List(self.parse_parameter_list(depth + 1)?)),
            Token::Name(name) => {
                let Some(next) = self.peek()? else {
                    return Err(StepError::syntax(
                        self.eof_span(),
                        "expected '(' after typed parameter name",
                    ));
                };
                if next.value != Token::OpenParen {
                    return Err(StepError::syntax(
                        next.span,
                        "expected '(' after typed parameter name",
                    ));
                }
                let _ = self.next()?;
                let base = self.parse_list_onto_scratch(depth + 1)?;
                // A single value is boxed straight off the stack, without
                // a one-element `Vec` built only to be taken apart again.
                let value = if self.scratch.len() == base + 1 {
                    Box::new(self.scratch.pop().expect("the list holds one value"))
                } else {
                    Box::new(Parameter::List(self.scratch.drain(base..).collect()))
                };
                Ok(Parameter::Typed {
                    type_name: S::name(name),
                    value,
                })
            }
            value => Err(StepError::syntax(
                token.span,
                format!("unexpected parameter token {value:?}"),
            )),
        }
    }

    fn expect_equals(&mut self) -> Result<(), StepError> {
        let token = self
            .next()?
            .ok_or_else(|| StepError::syntax(self.eof_span(), "expected '=' after instance id"))?;
        if token.value == Token::Equals {
            Ok(())
        } else {
            Err(StepError::syntax(
                token.span,
                "expected '=' after instance id",
            ))
        }
    }

    fn expect_semicolon(&mut self, context: &str) -> Result<(), StepError> {
        let token = self
            .next()?
            .ok_or_else(|| StepError::syntax(self.eof_span(), format!("expected ';' {context}")))?;
        if token.value == Token::Semicolon {
            Ok(())
        } else {
            Err(StepError::syntax(
                token.span,
                format!("expected ';' {context}"),
            ))
        }
    }

    fn next(&mut self) -> Result<Option<Spanned<Token<'a>>>, StepError> {
        let token = match self.lookahead.take() {
            Some(token) => Some(token),
            None => self.lexer.next_spanned()?,
        };
        if let Some(token) = &token {
            self.last_end = token.span.end;
        }
        Ok(token)
    }

    fn peek(&mut self) -> Result<Option<&Spanned<Token<'a>>>, StepError> {
        if self.lookahead.is_none() {
            self.lookahead = self.lexer.next_spanned()?;
        }
        Ok(self.lookahead.as_ref())
    }

    fn eof_span(&self) -> Span {
        let offset = self.last_end.max(self.lexer.offset());
        Span::new(offset, offset)
    }
}

fn upper(bytes: &[u8]) -> String {
    // Names reach here from the lexer, which admits only ASCII, so the lossy
    // conversion never replaces anything; the fast path skips its scan.
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_ascii_uppercase(),
        Err(_) => String::from_utf8_lossy(bytes).to_ascii_uppercase(),
    }
}

/// A lexeme as an owned `String`, reusing an owned buffer instead of copying.
/// Numbers and binaries are ASCII, so the lossy fallback never fires on them.
fn lexeme_string(bytes: Cow<'_, [u8]>) -> String {
    match bytes {
        Cow::Borrowed(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => String::from_utf8_lossy(bytes).into_owned(),
        },
        Cow::Owned(bytes) => String::from_utf8(bytes)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned()),
    }
}
