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
///
/// Performance note: the per-token helpers (`lex_id`, `lex_name`,
/// `lex_number`, ...) are `#[inline(always)]` into `next_spanned`. Out of
/// line, each returns its `Result<Token>` through a stack slot that
/// `next_spanned` then copies into its own result -- about 20% of lexing
/// cycles on real IFC files. Plain `#[inline]` is not enough: LLVM keeps
/// them out of line and the copy returns (measured with `perf`, 0.7.x).
/// The byte-class table (`CLASS`) replaces chains of range compares in the
/// per-byte loops.
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    input: &'a [u8],
    position: usize,
    finished: bool,
    /// Whether an ignored control was consumed since the current number
    /// token started. Lets [`Self::number_bytes`] borrow the lexeme without
    /// rescanning it; only `lex_number` resets and reads it.
    dirty: bool,
}

fn is_valid_binary(body: &[u8]) -> bool {
    matches!(body.first(), Some(b'0'..=b'3')) && body[1..].iter().all(u8::is_ascii_hexdigit)
}

const fn is_ignored_control(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | b'\r' | 0x0c)
}

// Byte classes, one table lookup instead of a chain of range compares in
// the per-byte loops. A byte may be in several classes.
/// ASCII whitespace as `u8::is_ascii_whitespace` defines it.
const WHITESPACE: u8 = 1;
/// An ignored control (`is_ignored_control`).
const CONTROL: u8 = 2;
/// `0-9`.
const DIGIT: u8 = 4;
/// Continues a bare name: ASCII alphanumeric, `_` or `-`.
const NAME: u8 = 8;

const fn byte_classes() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut b: u8 = 0;
    loop {
        let mut class = 0;
        if b.is_ascii_whitespace() {
            class |= WHITESPACE;
        }
        if is_ignored_control(b) {
            class |= CONTROL;
        }
        if b.is_ascii_digit() {
            class |= DIGIT;
        }
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
            class |= NAME;
        }
        table[b as usize] = class;
        if b == u8::MAX {
            break;
        }
        b += 1;
    }
    table
}

static CLASS: [u8; 256] = byte_classes();

#[inline(always)]
#[allow(clippy::inline_always, reason = "measured, see the note on `Lexer`")]
fn class(byte: u8) -> u8 {
    CLASS[byte as usize]
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

fn strip_text_print_directives(bytes: Cow<'_, [u8]>) -> Cow<'_, [u8]> {
    // Every directive and escape starts with `\`; most string bodies have none.
    if memchr::memchr(b'\\', &bytes).is_none() {
        return bytes;
    }
    let mut stripped: Option<Vec<u8>> = None;
    let mut position = 0;
    while position < bytes.len() {
        if bytes.get(position..position + 2) == Some(b"\\\\") {
            if let Some(output) = &mut stripped {
                output.extend_from_slice(b"\\\\");
            }
            position += 2;
        } else if matches!(bytes.get(position..position + 3), Some(b"\\N\\" | b"\\F\\")) {
            stripped.get_or_insert_with(|| {
                let mut output = Vec::with_capacity(bytes.len());
                output.extend_from_slice(&bytes[..position]);
                output
            });
            position += 3;
        } else {
            if let Some(output) = &mut stripped {
                output.push(bytes[position]);
            }
            position += 1;
        }
    }
    stripped.map_or(bytes, Cow::Owned)
}

impl<'a> Lexer<'a> {
    /// Starts tokenizing `input`.
    #[must_use]
    pub const fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            position: 0,
            finished: false,
            dirty: false,
        }
    }

    /// Current byte offset.
    #[must_use]
    pub const fn offset(&self) -> usize {
        self.position
    }

    /// Restarts tokenization at `position`.
    ///
    /// Only bounded error recovery uses this: after a lexical failure the
    /// parser must step past the damaged bytes to resynchronize. Callers are
    /// responsible for advancing monotonically, otherwise recovery can loop.
    pub(crate) fn resume_at(&mut self, position: usize) {
        self.position = position.min(self.input.len());
        self.finished = false;
    }

    fn skip_ignored_controls(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(|byte| is_ignored_control(*byte))
        {
            self.position += 1;
            self.dirty = true;
        }
    }

    /// Consumes a run of digits, with ignored controls allowed anywhere in
    /// it, and returns how many digits it held. Stops at the first byte that
    /// is neither, so it consumes exactly what alternating
    /// `skip_ignored_controls` + one-digit steps would.
    #[inline(always)]
    #[allow(clippy::inline_always, reason = "measured, see the note on `Lexer`")]
    fn digits(&mut self) -> usize {
        let mut count = 0;
        while let Some(&byte) = self.input.get(self.position) {
            let class = class(byte);
            if class & DIGIT != 0 {
                count += 1;
            } else if class & CONTROL != 0 {
                self.dirty = true;
            } else {
                break;
            }
            self.position += 1;
        }
        count
    }

    /// The number lexeme `start..self.position` with ignored controls
    /// removed. Borrows unless `digits`/`skip_ignored_controls` saw one.
    fn number_bytes(&self, start: usize) -> Cow<'a, [u8]> {
        if self.dirty {
            self.token_bytes(start, self.position)
        } else {
            Cow::Borrowed(&self.input[start..self.position])
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
    #[inline]
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

    #[inline]
    fn skip_trivia(&mut self) -> Result<(), StepError> {
        if self.position == 0 && self.input.starts_with(&[0xef, 0xbb, 0xbf]) {
            self.position = 3;
        }
        loop {
            while self
                .input
                .get(self.position)
                .is_some_and(|byte| class(*byte) & WHITESPACE != 0)
            {
                self.position += 1;
            }
            // The whitespace loop above already consumed every ignored
            // control, so a directive (`\N\`, `\F\`) or comment (`/*`) can
            // only start right here, at `\` or `/`. Anything else is a token.
            if !matches!(self.input.get(self.position), Some(b'\\' | b'/')) {
                return Ok(());
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

    #[inline(always)]
    #[allow(clippy::inline_always, reason = "measured, see the note on `Lexer`")]
    fn single(&mut self, token: Token<'a>) -> Token<'a> {
        self.position += 1;
        token
    }

    #[inline(always)]
    #[allow(clippy::inline_always, reason = "measured, see the note on `Lexer`")]
    fn lex_id(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        self.position += 1;
        self.skip_ignored_controls();
        // Controls before the first digit are outside the lexeme.
        self.dirty = false;
        let digits = self.position;
        if self.digits() == 0 {
            return Err(StepError::syntax(
                Span::new(start, self.position),
                "expected digits after '#'",
            ));
        }
        Ok(Token::Id(self.number_bytes(digits)))
    }

    fn lex_text(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        self.position += 1;
        let body_start = self.position;
        // Only `\` and `'` can change how a string body is read. Every other
        // byte -- ignored controls included, because the matchers below skip
        // those themselves before looking for `\` -- only advances by one, so
        // the scan jumps straight to the next candidate.
        while let Some(offset) = memchr::memchr2(b'\\', b'\'', &self.input[self.position..]) {
            self.position += offset;
            let byte = self.input[self.position];
            // `\\` is one escaped backslash. Consume it whole so its second
            // byte cannot open a `\S\` page escape below.
            if let Some(end) = self.match_ignoring_text_controls(self.position, br"\\") {
                self.position = end;
                continue;
            }
            // `\S\` takes exactly one following LATIN_CODEPOINT, and
            // APOSTROPHE is one (ISO 10303-21:2016 §5.2, §6.4.3.1). That byte
            // is payload even when it is `'`, so it never ends the literal.
            if let Some(end) = self.match_ignoring_text_controls(self.position, br"\S\") {
                self.position = end;
                self.skip_ignored_controls();
                if self.position < self.input.len() {
                    self.position += 1;
                }
                continue;
            }
            if byte == b'\'' {
                if let Some(end) = self.match_ignoring_text_controls(self.position, b"''") {
                    self.position = end;
                    continue;
                }
                let text = strip_text_print_directives(self.token_bytes(body_start, self.position));
                self.position += 1;
                return Ok(Token::Text(text));
            }
            self.position += 1;
        }
        self.position = self.input.len();
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

    #[inline(always)]
    #[allow(clippy::inline_always, reason = "measured, see the note on `Lexer`")]
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

    #[inline(always)]
    #[allow(clippy::inline_always, reason = "measured, see the note on `Lexer`")]
    fn lex_name(&mut self) -> Token<'a> {
        let start = self.position;
        let mut dirty = false;
        while let Some(&byte) = self.input.get(self.position) {
            let class = class(byte);
            if class & NAME != 0 {
                self.position += 1;
            } else if class & CONTROL != 0 {
                dirty = true;
                self.position += 1;
            } else {
                break;
            }
        }
        Token::Name(if dirty {
            self.token_bytes(start, self.position)
        } else {
            Cow::Borrowed(&self.input[start..self.position])
        })
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

    #[inline(always)]
    #[allow(clippy::inline_always, reason = "measured, see the note on `Lexer`")]
    fn lex_number(&mut self, start: usize) -> Result<Token<'a>, StepError> {
        // Ignored controls may appear anywhere inside a number and are
        // dropped from the lexeme. `dirty` records whether any was seen, so
        // the common clean number is borrowed without a second scan.
        self.dirty = false;
        if matches!(self.input.get(self.position), Some(b'+' | b'-')) {
            self.position += 1;
        }
        if self.digits() == 0 {
            return Err(StepError::syntax(
                Span::new(start, self.position),
                "number has no leading digits",
            ));
        }
        let mut real = false;
        if self.input.get(self.position) == Some(&b'.') {
            real = true;
            self.position += 1;
            self.digits();
        }
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
            }
            if self.digits() == 0 {
                return Err(StepError::syntax(
                    Span::new(start, self.position),
                    "real exponent has no digits",
                ));
            }
        }
        let text = self.number_bytes(start);
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
