//! Semantic parser and event/sink interface.
//!
//! Parsing is structural: record names and parameters are retained without a
//! domain schema. Unknown header and data records therefore survive a
//! parse/write/reparse cycle.

use crate::escape;
use crate::lexer::{Lexer, Token};
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
    parse_events(input, &mut builder)?;
    Ok(Exchange {
        header: builder.header,
        data: builder.data,
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
    if !crate::is_step_file(input) {
        return Err(StepError::not_step("missing ISO-10303-21 marker"));
    }
    Parser::new(input).parse(sink)
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
    lexer: Lexer<'a>,
    lookahead: Option<Spanned<Token<'a>>>,
    last_end: usize,
    phase: Phase,
    header_records_seen: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            lexer: Lexer::new(input),
            lookahead: None,
            last_end: 0,
            phase: Phase::BeforeStart,
            header_records_seen: 0,
        }
    }

    // Keeping section dispatch together makes the state-machine transitions auditable.
    #[allow(clippy::too_many_lines)]
    fn parse(&mut self, sink: &mut impl EventSink) -> Result<(), StepError> {
        while let Some(token) = self.next()? {
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
                    self.expect_equals()?;
                    let record_token = self.next()?.ok_or_else(|| {
                        StepError::syntax(
                            Span::new(token.span.end, token.span.end),
                            "missing record body",
                        )
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
                    sink.event(Event::DataRecord(DataRecord {
                        id: InstanceId::new(
                            std::str::from_utf8(&id).expect("instance digits are ASCII"),
                        )
                        .expect("lexer validates instance ids"),
                        records,
                    }));
                }
                _ => {
                    return Err(StepError::syntax(
                        token.span,
                        format!("unexpected token {:?} in {:?}", token.value, self.phase),
                    ));
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
