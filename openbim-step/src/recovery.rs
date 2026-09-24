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

use crate::{Exchange, InstanceId, Span};
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
    /// Whether to report duplicate instance ids and references to ids that
    /// are never defined. Off by default: strict parsing means syntax only.
    pub check_references: bool,
}

impl ParseOptions {
    /// Strict options: any malformed record aborts the parse.
    #[must_use]
    pub const fn strict() -> Self {
        Self {
            on_malformed_record: OnMalformed::Abort,
            check_references: false,
        }
    }

    /// Options that skip and report malformed data records.
    #[must_use]
    pub const fn lenient() -> Self {
        Self {
            on_malformed_record: OnMalformed::Skip,
            check_references: false,
        }
    }

    /// Sets the malformed-record policy.
    #[must_use]
    pub const fn on_malformed_record(mut self, policy: OnMalformed) -> Self {
        self.on_malformed_record = policy;
        self
    }

    /// Enables or disables reference-integrity diagnostics.
    ///
    /// When enabled, every data record whose instance id was already defined
    /// yields a [`DiagnosticKind::DuplicateId`], and every record referencing
    /// an id that no record in the `DATA` section defines yields one
    /// [`DiagnosticKind::DanglingReference`] per distinct missing id. Forward
    /// references are legal (ISO 10303-21:2016 §11.2) and are only reported
    /// if the target is still undefined at `ENDSEC`. Ids compare numerically,
    /// so `#07` and `#7` name the same instance.
    ///
    /// Nothing is dropped or rewritten, so these diagnostics never make an
    /// outcome lossy. The check keeps one entry per defined id, so memory is
    /// linear in the number of records, including for [`crate::parse_events_with`].
    #[must_use]
    pub const fn check_references(mut self, enabled: bool) -> Self {
        self.check_references = enabled;
        self
    }
}

/// Severity of a non-fatal parse diagnostic.
///
/// Only [`Severity::Warning`] exists today: every diagnostic describes input
/// that was accepted. Fatal problems are returned as
/// [`StepError`](crate::StepError) instead of being reported here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Severity {
    /// Input was accepted, but is damaged or inconsistent.
    #[default]
    Warning,
}

/// What a [`Diagnostic`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// A malformed data record was skipped under [`OnMalformed::Skip`]. The
    /// only kind that loses input.
    SkippedRecord,
    /// A data record reuses an instance id defined earlier in the section.
    /// ISO 10303-21:2016 §11.2 requires instance names to be unique. Both
    /// records are kept; the diagnostic points at the later one.
    DuplicateId,
    /// A data record references an instance id that no record defines.
    DanglingReference,
}

/// A non-fatal problem found while parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    severity: Severity,
    kind: DiagnosticKind,
    span: Span,
    instance: Option<InstanceId>,
    detail: String,
}

impl Diagnostic {
    pub(crate) fn skipped_record(span: Span, detail: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            kind: DiagnosticKind::SkippedRecord,
            span,
            instance: None,
            detail: detail.into(),
        }
    }

    pub(crate) fn duplicate_id(span: Span, id: InstanceId) -> Self {
        Self {
            severity: Severity::Warning,
            kind: DiagnosticKind::DuplicateId,
            span,
            detail: format!("duplicate instance id {id}"),
            instance: Some(id),
        }
    }

    pub(crate) fn dangling_reference(span: Span, id: InstanceId) -> Self {
        Self {
            severity: Severity::Warning,
            kind: DiagnosticKind::DanglingReference,
            span,
            detail: format!("reference to undefined instance {id}"),
            instance: Some(id),
        }
    }

    /// Severity of the diagnostic.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// What the diagnostic reports.
    #[must_use]
    pub const fn kind(&self) -> DiagnosticKind {
        self.kind
    }

    /// The instance id at fault. For a duplicate it is the later record's id
    /// as written; for a dangling reference it is the missing id in canonical
    /// form (leading zeros removed, since `#07` and `#7` are the same
    /// instance). `None` for a skipped record.
    #[must_use]
    pub const fn instance(&self) -> Option<&InstanceId> {
        self.instance.as_ref()
    }

    /// Byte range of the original input that the diagnostic covers.
    ///
    /// For a skipped record this is the whole discarded range, so a consumer
    /// can quote the exact bytes that were dropped. For a reference defect it
    /// is the offending record: the later duplicate, or the record holding the
    /// dangling reference.
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
    /// Whether nothing was dropped while reading.
    ///
    /// Only [`DiagnosticKind::SkippedRecord`] loses input. Reference
    /// defects are reported but keep every record, so they do not count.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::SkippedRecord)
    }
}
