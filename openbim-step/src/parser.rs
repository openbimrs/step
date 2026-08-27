//! Semantic parser and event/sink interface.
//!
//! Parsing is structural: record names and parameters are retained without a
//! domain schema. Unknown header and data records therefore survive a
//! parse/write/reparse cycle.

use crate::escape;
use crate::lexer::{Lexer, Token};
use crate::recovery::{Diagnostic, OnMalformed, ParseOptions, ParseOutcome};
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
    parser.parse(sink)?;
    Ok(parser.diagnostics)
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

struct Parser<'a> {
    input: &'a [u8],
    lexer: Lexer<'a>,
    lookahead: Option<Spanned<Token<'a>>>,
    last_end: usize,
    phase: Phase,
    header_records_seen: usize,
    options: ParseOptions,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
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
        }
    }

    // Keeping section dispatch together makes the state-machine transitions auditable.
    #[allow(clippy::too_many_lines)]
    fn parse(&mut self, sink: &mut impl EventSink) -> Result<(), StepError> {
        loop {
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
                        name: upper(&name),
                        parameters,
                    }));
                }
                Token::Id(id) if self.phase == Phase::Data => {
                    let start = token.span.start;
                    match self.parse_data_record(&id, token.span) {
                        Ok(record) => sink.event(Event::DataRecord(record)),
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
    fn parse_data_record(
        &mut self,
        id: &[u8],
        id_span: Span,
    ) -> Result<DataRecord<String>, StepError> {
        self.expect_equals()?;
        let record_token = self.next()?.ok_or_else(|| {
            StepError::syntax(Span::new(id_span.end, id_span.end), "missing record body")
        })?;
        let records = match record_token.value {
            Token::Name(name) => vec![self.parse_named_record(&name)?],
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
                    records.push(self.parse_named_record(&name)?);
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
            id: InstanceId::new(std::str::from_utf8(id).expect("instance digits are ASCII"))
                .expect("lexer validates instance ids"),
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
    /// failed. Quoted strings and comments are tracked so a `;` or `ENDSEC`
    /// inside a literal is not mistaken for a record boundary. A section
    /// terminator stops the scan *before* it is consumed, so recovery can
    /// never swallow the end of `DATA`.
    fn resync_from(&self, from: usize) -> usize {
        let mut position = from.min(self.input.len());
        let mut in_string = false;
        while position < self.input.len() {
            let byte = self.input[position];
            if in_string {
                if byte == b'\'' {
                    if self.input.get(position + 1) == Some(&b'\'') {
                        position += 2;
                        continue;
                    }
                    in_string = false;
                }
                position += 1;
                continue;
            }
            match byte {
                b'\'' => {
                    in_string = true;
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

    fn parse_named_record(&mut self, name: &[u8]) -> Result<Record, StepError> {
        Ok(Record {
            name: upper(name),
            parameters: self.parse_arguments()?,
        })
    }

    fn parse_arguments(&mut self) -> Result<Vec<Parameter>, StepError> {
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

    fn parse_parameter_list(&mut self, depth: usize) -> Result<Vec<Parameter>, StepError> {
        if depth > crate::MAX_PARAMETER_NESTING {
            let span = match self.peek()? {
                Some(token) => token.span,
                None => self.eof_span(),
            };
            return Err(StepError::syntax(span, "parameter nesting limit exceeded"));
        }
        let mut parameters = Vec::new();
        if self
            .peek()?
            .is_some_and(|token| token.value == Token::CloseParen)
        {
            let _ = self.next()?;
            return Ok(parameters);
        }
        loop {
            parameters.push(self.parse_parameter(depth)?);
            let separator = self
                .next()?
                .ok_or_else(|| StepError::syntax(self.eof_span(), "unterminated parameter list"))?;
            match separator.value {
                Token::Comma => {}
                Token::CloseParen => return Ok(parameters),
                _ => {
                    return Err(StepError::syntax(
                        separator.span,
                        "expected ',' or ')' after parameter",
                    ));
                }
            }
        }
    }

    fn parse_parameter(&mut self, depth: usize) -> Result<Parameter, StepError> {
        let token = self
            .next()?
            .ok_or_else(|| StepError::syntax(self.eof_span(), "expected parameter"))?;
        match token.value {
            Token::Dollar => Ok(Parameter::Null),
            Token::Star => Ok(Parameter::Derived),
            Token::Id(id) => Ok(Parameter::Ref(
                InstanceId::new(std::str::from_utf8(&id).expect("instance digits are ASCII"))
                    .expect("lexer validates instance ids"),
            )),
            Token::Integer(value) => Ok(Parameter::Integer(
                String::from_utf8_lossy(&value).into_owned(),
            )),
            Token::Real(value) => Ok(Parameter::Real(
                String::from_utf8_lossy(&value).into_owned(),
            )),
            Token::Text(raw) => Ok(Parameter::Text(escape::decode(&raw))),
            Token::Binary(raw) => Ok(Parameter::Binary(
                String::from_utf8_lossy(&raw).into_owned(),
            )),
            Token::Keyword(keyword) if keyword.eq_ignore_ascii_case(b"T") => {
                Ok(Parameter::Bool(true))
            }
            Token::Keyword(keyword) if keyword.eq_ignore_ascii_case(b"F") => {
                Ok(Parameter::Bool(false))
            }
            Token::Keyword(keyword) if keyword.eq_ignore_ascii_case(b"U") => {
                Ok(Parameter::LogicalUnknown)
            }
            Token::Keyword(keyword) => Ok(Parameter::Enum(upper(&keyword))),
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
                let mut parameters = self.parse_parameter_list(depth + 1)?;
                let value = if parameters.len() == 1 {
                    Box::new(parameters.remove(0))
                } else {
                    Box::new(Parameter::List(parameters))
                };
                Ok(Parameter::Typed {
                    type_name: upper(&name),
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
    String::from_utf8_lossy(bytes).to_ascii_uppercase()
}
