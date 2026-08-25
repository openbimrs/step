//! Byte-level tokenizer for STEP physical files.
//!
//! The tokenizer accepts arbitrary bytes because the file structure is ASCII
//! and legacy producers may place non-UTF-8 bytes inside string literals.

use crate::{Span, Spanned, StepError};

/// One lexical unit in a physical file.
#[derive(Debug, Clone, PartialEq)]
pub enum Token<'a> {
    /// `#42`.
    Id(&'a [u8]),
    /// A quoted string body, still escaped.
    Text(&'a [u8]),
    /// A binary literal body.
    Binary(&'a [u8]),
    /// A dotted keyword or enumeration name, without dots.
    Keyword(&'a [u8]),
    /// A bare identifier or physical-file marker.
    Name(&'a [u8]),
    /// Integer lexical bytes.
    Integer(&'a [u8]),
    /// Real lexical bytes.
    Real(&'a [u8]),
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

    /// Produces the next spanned token.
    ///
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
            value if value.is_ascii_alphabetic() => self.lex_name(),
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
            if self.input[self.position..].starts_with(b"/*") {
                let start = self.position;
                let Some(end) = self.input[self.position + 2..]
                    .windows(2)
                    .position(|window| window == b"*/")
                else {
                    self.position = self.input.len();
                    return Err(StepError::syntax(
                        Span::new(start, self.position),
                        "unterminated comment",
                    ));
                };
                self.position += 2 + end + 2;
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
        let digits = self.position;
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_digit)
        {
            self.position += 1;
        }
        if digits == self.position {
            return Err(StepError::syntax(
                Span::new(start, self.position),
                "expected digits after '#'",
            ));
        }
        Ok(Token::Id(&self.input[digits..self.position]))
    }

    fn lex_text(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        self.position += 1;
        let body_start = self.position;
        while let Some(&byte) = self.input.get(self.position) {
            if byte == b'\'' {
                if self.input.get(self.position + 1) == Some(&b'\'') {
                    self.position += 2;
                    continue;
                }
                let text = &self.input[body_start..self.position];
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
                let body = &self.input[body_start..self.position];
                self.position += 1;
                if !is_valid_binary(body) {
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
            if byte == b'.' {
                if self.position == body_start {
                    self.position += 1;
                    return Err(StepError::syntax(
                        Span::new(start, self.position),
                        "empty dotted keyword",
                    ));
                }
                let body = &self.input[body_start..self.position];
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
        while self
            .input
            .get(self.position)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            self.position += 1;
        }
        Token::Name(&self.input[start..self.position])
    }

    fn lex_user_defined_name(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        self.position += 1;
        let body_start = self.position;
        if !self
            .input
            .get(body_start)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            return Err(StepError::syntax(
                Span::new(start, self.position),
                "invalid user-defined keyword",
            ));
        }
        self.position += 1;
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_alphanumeric)
        {
            self.position += 1;
        }
        if matches!(self.input.get(self.position), Some(b'_' | b'-')) {
            return Err(StepError::syntax(
                Span::new(start, self.position + 1),
                "invalid user-defined keyword",
            ));
        }
        Ok(Token::Name(&self.input[start..self.position]))
    }

    fn lex_number(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        if matches!(self.input.get(self.position), Some(b'+' | b'-')) {
            self.position += 1;
        }
        let integer_start = self.position;
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_digit)
        {
            self.position += 1;
        }
        if self.position == integer_start {
            return Err(StepError::syntax(
                Span::new(start, self.position),
                "number has no leading digits",
            ));
        }
        let mut real = false;
        if self.input.get(self.position) == Some(&b'.') {
            real = true;
            self.position += 1;
            while self
                .input
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
        }
        if matches!(self.input.get(self.position), Some(b'e' | b'E')) {
            if !real {
                return Err(StepError::syntax(
                    Span::new(start, self.position + 1),
                    "real requires a decimal point before its exponent",
                ));
            }
            self.position += 1;
            if matches!(self.input.get(self.position), Some(b'+' | b'-')) {
                self.position += 1;
            }
            let exponent_start = self.position;
            while self
                .input
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
            if exponent_start == self.position {
                return Err(StepError::syntax(
                    Span::new(start, self.position),
                    "real exponent has no digits",
                ));
            }
        }
        let text = &self.input[start..self.position];
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
