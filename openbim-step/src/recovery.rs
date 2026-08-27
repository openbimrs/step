//! Opt-in malformed-record recovery policy and non-fatal diagnostics.
//!
//! Strict parsing is the default because an authoring tool that silently drops
//! data corrupts the model it is editing. A consumer (viewer, importer,
//! reporter) has the opposite need: real exporter output contains occasional
//! damaged records, and refusing an entire file over one of them is not useful.
//!
//! Recovery is therefore explicit, bounded, and reported: the caller opts in,
//! only data records are recoverable, and every skipped byte range comes back
//! as a [`Diagnostic`] so a consumer can show what was lost instead of
//! pretending the file was clean.

use crate::{Exchange, Span};
use std::fmt;

/// What to do when a data record cannot be parsed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum OnMalformed {
    /// Fail the parse. The default: an unreadable record is a hard error.
    #[default]
    Abort,
    /// Report the record as a diagnostic, resynchronize, and keep parsing.
    Skip,
}

/// Parse behavior toggles.
///
/// Constructed with [`ParseOptions::default`] (strict) and adjusted through
/// [`ParseOptions::on_malformed_record`], so later options cannot break
/// existing call sites.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ParseOptions {
    /// Policy for unparsable data records.
    pub on_malformed_record: OnMalformed,
}

impl ParseOptions {
    /// Strict options: any malformed record aborts the parse.
    #[must_use]
    pub const fn strict() -> Self {
        Self {
            on_malformed_record: OnMalformed::Abort,
        }
    }

    /// Options that skip and report malformed data records.
    #[must_use]
    pub const fn lenient() -> Self {
        Self {
            on_malformed_record: OnMalformed::Skip,
        }
    }

    /// Sets the malformed-record policy.
    #[must_use]
    pub const fn on_malformed_record(mut self, policy: OnMalformed) -> Self {
        self.on_malformed_record = policy;
        self
    }
}

/// Severity of a non-fatal parse diagnostic.
///
/// Only [`Severity::Warning`] exists today: a diagnostic is emitted exactly
/// when input was accepted but not fully represented. Fatal problems are
/// returned as [`StepError`](crate::StepError) instead of being reported here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Severity {
    /// Input was recovered with loss.
    #[default]
    Warning,
}

/// A non-fatal problem found while parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    severity: Severity,
    span: Span,
    detail: String,
}

impl Diagnostic {
    pub(crate) fn skipped_record(span: Span, detail: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            span,
            detail: detail.into(),
        }
    }

    /// Severity of the diagnostic.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Byte range of the original input that the diagnostic covers.
    ///
    /// For a skipped record this is the whole discarded range, so a consumer
    /// can quote the exact bytes that were dropped.
    #[must_use]
    pub const fn span(&self) -> Span {
        self.span
    }

    /// Human-readable description without a location prefix.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "STEP warning at bytes {}..{}: {}",
            self.span.start, self.span.end, self.detail
        )
    }
}

/// A parsed exchange together with everything that was recovered with loss.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseOutcome {
    /// Records that were read successfully.
    pub exchange: Exchange,
    /// Non-fatal problems, in source order. Empty for a clean file.
    pub diagnostics: Vec<Diagnostic>,
}

impl ParseOutcome {
    /// Whether anything was dropped while reading.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.diagnostics.is_empty()
    }
}
