//! Byte-level tokenizer for STEP physical files.
//!
//! The tokenizer accepts arbitrary bytes because the file structure is ASCII
//! and legacy producers may place non-UTF-8 bytes inside string literals.

use crate::{Span, Spanned, StepError};
use std::borrow::Cow;

/// One lexical unit in a physical file.
#[derive(Debug, Clone, PartialEq)]
pub enum Token<'a> {
    /// `#42`.
    Id(Cow<'a, [u8]>),
    /// A quoted string body, still escaped.
    Text(Cow<'a, [u8]>),
    /// A binary literal body.
    Binary(Cow<'a, [u8]>),
    /// A dotted keyword or enumeration name, without dots.
    Keyword(Cow<'a, [u8]>),
    /// A bare identifier or physical-file marker.
    Name(Cow<'a, [u8]>),
    /// Integer lexical bytes.
    Integer(Cow<'a, [u8]>),
    /// Real lexical bytes.
    Real(Cow<'a, [u8]>),
    /// `$`.
    Dollar,
    /// `*`.
    Star,
    /// `(`.
    OpenParen,
    /// `)`.
    CloseParen,
    /// `,`.
    Comma,
    /// `=`.
    Equals,
    /// `;`.
    Semicolon,
}

/// Streaming tokenizer over a byte slice.
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    input: &'a [u8],
    position: usize,
    finished: bool,
}

fn is_valid_binary(body: &[u8]) -> bool {
    matches!(body.first(), Some(b'0'..=b'3')) && body[1..].iter().all(u8::is_ascii_hexdigit)
}

const fn is_ignored_control(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | b'\r' | 0x0c)
}

fn strip_print_directives(bytes: Cow<'_, [u8]>) -> Cow<'_, [u8]> {
    if !bytes
        .windows(3)
        .any(|window| matches!(window, b"\\N\\" | b"\\F\\"))
    {
        return bytes;
    }
    let mut stripped = Vec::with_capacity(bytes.len());
    let mut position = 0;
    while position < bytes.len() {
        if matches!(bytes.get(position..position + 3), Some(b"\\N\\" | b"\\F\\")) {
            position += 3;
        } else {
            stripped.push(bytes[position]);
            position += 1;
        }
    }
    Cow::Owned(stripped)
}

impl<'a> Lexer<'a> {
    /// Starts tokenizing `input`.
    #[must_use]
    pub const fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            position: 0,
            finished: false,
        }
    }

    /// Current byte offset.
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.position
    }

    fn skip_ignored_controls(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(|byte| is_ignored_control(*byte))
        {
            self.position += 1;
        }
    }

    fn token_bytes(&self, start: usize, end: usize) -> Cow<'a, [u8]> {
        let bytes = &self.input[start..end];
        if bytes.iter().any(|byte| is_ignored_control(*byte)) {
            Cow::Owned(
                bytes
                    .iter()
                    .copied()
                    .filter(|byte| !is_ignored_control(*byte))
                    .collect(),
            )
        } else {
            Cow::Borrowed(bytes)
        }
    }

    fn match_ignoring_controls(&self, start: usize, expected: &[u8]) -> Option<usize> {
        let mut position = start;
        for &expected_byte in expected {
            while self
                .input
                .get(position)
                .is_some_and(|byte| is_ignored_control(*byte))
            {
                position += 1;
            }
            if self.input.get(position) != Some(&expected_byte) {
                return None;
            }
            position += 1;
        }
        Some(position)
    }

    fn match_ignoring_text_controls(&self, start: usize, expected: &[u8]) -> Option<usize> {
        let mut position = start;
        for &expected_byte in expected {
            loop {
                while self
                    .input
                    .get(position)
                    .is_some_and(|byte| is_ignored_control(*byte))
                {
                    position += 1;
                }
                let Some(end) = self
                    .match_ignoring_controls(position, b"\\N\\")
                    .or_else(|| self.match_ignoring_controls(position, b"\\F\\"))
                else {
                    break;
                };
                position = end;
            }
            if self.input.get(position) != Some(&expected_byte) {
                return None;
            }
            position += 1;
        }
        Some(position)
    }

    /// Produces the next spanned token.
    /// # Errors
    ///
    /// Returns a syntax diagnostic for malformed literals, comments, numbers,
    /// or bytes that are not STEP punctuation.
    pub fn next_spanned(&mut self) -> Result<Option<Spanned<Token<'a>>>, StepError> {
        if self.finished {
            return Ok(None);
        }
        self.skip_trivia()?;
        let Some(&byte) = self.input.get(self.position) else {
            self.finished = true;
            return Ok(None);
        };
        let start = self.position;
        let value = match byte {
            b'(' => self.single(Token::OpenParen),
            b')' => self.single(Token::CloseParen),
            b',' => self.single(Token::Comma),
            b'=' => self.single(Token::Equals),
            b';' => self.single(Token::Semicolon),
            b'$' => self.single(Token::Dollar),
            b'*' => self.single(Token::Star),
            b'#' => self.lex_id(start)?,
            b'!' => self.lex_user_defined_name(start)?,
            b'\'' => self.lex_text(start)?,
            b'"' => self.lex_binary(start)?,
            b'.' if self.input.get(start + 1).is_some_and(u8::is_ascii_digit) => {
                self.lex_number(start)?
            }
            b'.' => self.lex_keyword(start)?,
            b'0'..=b'9' | b'-' | b'+' => self.lex_number(start)?,
            value if value.is_ascii_alphabetic() || value == b'_' => self.lex_name(),
            _ => {
                self.position += 1;
                return Err(StepError::syntax(
                    Span::new(start, self.position),
                    format!("unexpected byte 0x{byte:02X}"),
                ));
            }
        };
        Ok(Some(Spanned::new(value, Span::new(start, self.position))))
    }

    /// Produces the next spanned token.
    ///
    /// This is an alias for [`Lexer::next_spanned`].
    ///
    /// # Errors
    ///
    /// Returns a syntax diagnostic for malformed input.
    pub fn next_token(&mut self) -> Result<Option<Spanned<Token<'a>>>, StepError> {
        self.next_spanned()
    }

    fn skip_trivia(&mut self) -> Result<(), StepError> {
        if self.position == 0 && self.input.starts_with(&[0xef, 0xbb, 0xbf]) {
            self.position = 3;
        }
        loop {
            while self
                .input
                .get(self.position)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.position += 1;
            }
            if let Some(end) = self
                .match_ignoring_controls(self.position, b"\\N\\")
                .or_else(|| self.match_ignoring_controls(self.position, b"\\F\\"))
            {
                self.position = end;
                continue;
            }
            if let Some(body_start) = self.match_ignoring_controls(self.position, b"/*") {
                let start = self.position;
                let mut cursor = body_start;
                loop {
                    if let Some(end) = self.match_ignoring_controls(cursor, b"*/") {
                        self.position = end;
                        break;
                    }
                    if cursor >= self.input.len() {
                        self.position = self.input.len();
                        return Err(StepError::syntax(
                            Span::new(start, self.position),
                            "unterminated comment",
                        ));
                    }
                    cursor += 1;
                }
                continue;
            }
            return Ok(());
        }
    }

    fn single(&mut self, token: Token<'a>) -> Token<'a> {
        self.position += 1;
        token
    }

    fn lex_id(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        self.position += 1;
        self.skip_ignored_controls();
        let digits = self.position;
        let mut saw_digit = false;
        while let Some(&byte) = self.input.get(self.position) {
            if is_ignored_control(byte) {
                self.position += 1;
            } else if byte.is_ascii_digit() {
                saw_digit = true;
                self.position += 1;
            } else {
                break;
            }
        }
        if !saw_digit {
            return Err(StepError::syntax(
                Span::new(start, self.position),
                "expected digits after '#'",
            ));
        }
        Ok(Token::Id(self.token_bytes(digits, self.position)))
    }

    fn lex_text(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        self.position += 1;
        let body_start = self.position;
        while let Some(&byte) = self.input.get(self.position) {
            if byte == b'\'' {
                if let Some(end) = self.match_ignoring_text_controls(self.position, b"''") {
                    self.position = end;
                    continue;
                }
                let text = strip_print_directives(self.token_bytes(body_start, self.position));
                self.position += 1;
                return Ok(Token::Text(text));
            }
            self.position += 1;
        }
        Err(StepError::syntax(
            Span::new(start, self.position),
            "unterminated string literal",
        ))
    }

    fn lex_binary(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        self.position += 1;
        let body_start = self.position;
        while let Some(&byte) = self.input.get(self.position) {
            if byte == b'"' {
                let body = strip_print_directives(self.token_bytes(body_start, self.position));
                self.position += 1;
                if !is_valid_binary(body.as_ref()) {
                    return Err(StepError::syntax(
                        Span::new(start, self.position),
                        "invalid binary literal",
                    ));
                }
                return Ok(Token::Binary(body));
            }
            self.position += 1;
        }
        Err(StepError::syntax(
            Span::new(start, self.position),
            "unterminated binary literal",
        ))
    }

    fn lex_keyword(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        self.position += 1;
        let body_start = self.position;
        while let Some(&byte) = self.input.get(self.position) {
            if is_ignored_control(byte) {
                self.position += 1;
                continue;
            }
            if byte == b'.' {
                let body = self.token_bytes(body_start, self.position);
                if body.is_empty() {
                    self.position += 1;
                    return Err(StepError::syntax(
                        Span::new(start, self.position),
                        "empty dotted keyword",
                    ));
                }
                self.position += 1;
                return Ok(Token::Keyword(body));
            }
            if !(byte.is_ascii_alphanumeric() || byte == b'_') {
                break;
            }
            self.position += 1;
        }
        Err(StepError::syntax(
            Span::new(start, self.position),
            "unterminated dotted keyword",
        ))
    }

    fn lex_name(&mut self) -> Token<'a> {
        let start = self.position;
        while let Some(&byte) = self.input.get(self.position) {
            if is_ignored_control(byte)
                || byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-')
            {
                self.position += 1;
            } else {
                break;
            }
        }
        Token::Name(self.token_bytes(start, self.position))
    }

    fn lex_user_defined_name(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        self.position += 1;
        self.skip_ignored_controls();
        let body_start = self.position;
        if !self
            .input
            .get(body_start)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        {
            return Err(StepError::syntax(
                Span::new(start, self.position),
                "invalid user-defined keyword",
            ));
        }
        self.position += 1;
        while let Some(&byte) = self.input.get(self.position) {
            if is_ignored_control(byte) || byte.is_ascii_alphanumeric() || byte == b'_' {
                self.position += 1;
            } else {
                break;
            }
        }
        if matches!(self.input.get(self.position), Some(b'-')) {
            return Err(StepError::syntax(
                Span::new(start, self.position + 1),
                "invalid user-defined keyword",
            ));
        }
        Ok(Token::Name(self.token_bytes(start, self.position)))
    }

    fn lex_number(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        if matches!(self.input.get(self.position), Some(b'+' | b'-')) {
            self.position += 1;
            self.skip_ignored_controls();
        }
        let mut integer_digits = 0;
        loop {
            self.skip_ignored_controls();
            if self
                .input
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                integer_digits += 1;
                self.position += 1;
            } else {
                break;
            }
        }
        if integer_digits == 0 {
            return Err(StepError::syntax(
                Span::new(start, self.position),
                "number has no leading digits",
            ));
        }
        self.skip_ignored_controls();
        let mut real = false;
        if self.input.get(self.position) == Some(&b'.') {
            real = true;
            self.position += 1;
            loop {
                self.skip_ignored_controls();
                if self
                    .input
                    .get(self.position)
                    .is_some_and(u8::is_ascii_digit)
                {
                    self.position += 1;
                } else {
                    break;
                }
            }
        }
        self.skip_ignored_controls();
        if matches!(self.input.get(self.position), Some(b'e' | b'E')) {
            if !real {
                return Err(StepError::syntax(
                    Span::new(start, self.position + 1),
                    "real requires a decimal point before its exponent",
                ));
            }
            self.position += 1;
            self.skip_ignored_controls();
            if matches!(self.input.get(self.position), Some(b'+' | b'-')) {
                self.position += 1;
                self.skip_ignored_controls();
            }
            let mut exponent_digits = 0;
            loop {
                self.skip_ignored_controls();
                if self
                    .input
                    .get(self.position)
                    .is_some_and(u8::is_ascii_digit)
                {
                    exponent_digits += 1;
                    self.position += 1;
                } else {
                    break;
                }
            }
            if exponent_digits == 0 {
                return Err(StepError::syntax(
                    Span::new(start, self.position),
                    "real exponent has no digits",
                ));
            }
        }
        let text = self.token_bytes(start, self.position);
        Ok(if real {
            Token::Real(text)
        } else {
            Token::Integer(text)
        })
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Spanned<Token<'a>>, StepError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_spanned() {
            Ok(Some(token)) => Some(Ok(token)),
            Ok(None) => None,
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }
}
