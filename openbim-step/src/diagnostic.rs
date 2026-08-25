//! Source spans and diagnostics for ISO 10303-21 input.

use std::error::Error;
use std::fmt;

/// A half-open byte range in the original source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Span {
    /// Inclusive byte offset.
    pub start: usize,
    /// Exclusive byte offset.
    pub end: usize,
}

impl Span {
    /// Creates a half-open source span.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Returns the number of bytes covered by this span.
    #[must_use]
    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Returns whether this span is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

/// A value paired with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    /// Parsed or tokenized value.
    pub value: T,
    /// Byte range from which the value came.
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Pairs a value with a source span.
    #[must_use]
    pub const fn new(value: T, span: Span) -> Self {
        Self { value, span }
    }
}

/// Source text used to resolve byte spans to human-readable locations.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    name: &'a str,
    bytes: &'a [u8],
}

impl<'a> Source<'a> {
    /// Creates a named source view.
    #[must_use]
    pub const fn new(name: &'a str, bytes: &'a [u8]) -> Self {
        Self { name, bytes }
    }

    /// Resolves the start of a span to a one-based line and column.
    #[must_use]
    pub fn location(self, span: Span) -> SourceLocation<'a> {
        let offset = span.start.min(self.bytes.len());
        let prefix = &self.bytes[..offset];
        let line = prefix
            .iter()
            .fold(1, |line, byte| line + usize::from(*byte == b'\n'));
        let line_start = prefix
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1);
        let line_end = self.bytes[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(self.bytes.len(), |position| line_start + position);
        let column = String::from_utf8_lossy(&self.bytes[line_start..offset])
            .chars()
            .count()
            + 1;
        SourceLocation {
            source_name: self.name,
            line,
            column,
            line_text: String::from_utf8_lossy(&self.bytes[line_start..line_end]).into_owned(),
            span,
        }
    }
}

/// A source span resolved to a display location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation<'a> {
    /// Name supplied to [`Source::new`].
    pub source_name: &'a str,
    /// One-based line number.
    pub line: usize,
    /// One-based Unicode scalar column.
    pub column: usize,
    /// Lossily decoded source line for rendering a diagnostic.
    pub line_text: String,
    /// Original byte span.
    pub span: Span,
}

/// Failure while tokenizing or semantically parsing a physical file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepError {
    span: Span,
    detail: String,
    kind: ErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorKind {
    NotStep,
    Syntax,
    InvalidArgument,
}

impl StepError {
    pub(crate) fn not_step(detail: impl Into<String>) -> Self {
        Self {
            span: Span::new(0, 0),
            detail: detail.into(),
            kind: ErrorKind::NotStep,
        }
    }

    pub(crate) fn syntax(span: Span, detail: impl Into<String>) -> Self {
        Self {
            span,
            detail: detail.into(),
            kind: ErrorKind::Syntax,
        }
    }

    pub(crate) fn invalid_argument(detail: impl Into<String>) -> Self {
        Self {
            span: Span::new(0, 0),
            detail: detail.into(),
            kind: ErrorKind::InvalidArgument,
        }
    }

    /// Returns whether the input lacked the ISO 10303-21 physical-file marker.
    #[must_use]
    pub const fn is_not_step(&self) -> bool {
        matches!(self.kind, ErrorKind::NotStep)
    }

    /// Byte span associated with the failure.
    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    /// Diagnostic detail without the location prefix.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for StepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ErrorKind::NotStep => write!(formatter, "not a STEP physical file: {}", self.detail),
            ErrorKind::Syntax => write!(
                formatter,
                "STEP syntax error at bytes {}..{}: {}",
                self.span.start, self.span.end, self.detail
            ),
            ErrorKind::InvalidArgument => write!(formatter, "invalid argument: {}", self.detail),
        }
    }
}

impl Error for StepError {}
