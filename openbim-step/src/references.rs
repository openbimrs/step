//! Opt-in reference-integrity check over the `DATA` section.
//!
//! Runs inside the parser so the owned and the streaming APIs report the
//! same thing, and so no second pass over the records is needed. Enabled by
//! [`ParseOptions::check_references`](crate::ParseOptions::check_references).
//!
//! Cost model: one hash-set entry per defined id, and one [`Key`] per
//! reference that is not yet defined when its record is read. STEP exporters
//! commonly write top-down, so most references are forward references; the
//! per-reference work is therefore kept allocation-free (a `u64` key, no
//! per-record scratch collections). Deduplication happens at `ENDSEC`, and
//! only for records that still have missing ids -- none, in a valid file.
//!
//! The hasher is std's randomly keyed `SipHash` on purpose: instance ids are
//! attacker-controlled input, and a fixed-key hasher would expose the check
//! to hash flooding.

use crate::{DataRecord, Diagnostic, InstanceId, Parameter, Span};
use std::collections::HashSet;

/// An instance name as a number, for comparison: `#007` and `#7` are the
/// same instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    /// The id fits in 64 bits, as every id real exporters write does.
    Small(u64),
    /// Canonical decimal digits of a larger id. Part 21 does not bound
    /// instance names, so these must still compare correctly.
    Large(Box<str>),
}

impl Key {
    fn of(id: &InstanceId) -> Self {
        let digits = id.as_str().trim_start_matches('0');
        if digits.is_empty() {
            return Self::Small(0);
        }
        digits
            .parse()
            .map_or_else(|_| Self::Large(digits.into()), Self::Small)
    }

    /// The canonical instance id this key stands for.
    fn to_id(&self) -> InstanceId {
        match self {
            Self::Small(value) => InstanceId::from(*value),
            Self::Large(digits) => {
                InstanceId::new(digits).expect("canonical digits are a valid instance id")
            }
        }
    }
}

/// Defined ids, split by representation so the common case is an 8-byte
/// key: at a million records, a 24-byte enum key measurably slowed the
/// check through larger rehash copies.
#[derive(Debug, Default)]
struct Defined {
    small: HashSet<u64>,
    large: HashSet<Box<str>>,
}

impl Defined {
    /// Adds `key`; returns whether it was new.
    fn insert(&mut self, key: Key) -> bool {
        match key {
            Key::Small(value) => self.small.insert(value),
            Key::Large(digits) => self.large.insert(digits),
        }
    }

    fn contains(&self, key: &Key) -> bool {
        match key {
            Key::Small(value) => self.small.contains(value),
            Key::Large(digits) => self.large.contains(digits),
        }
    }
}

/// Records seen so far, plus the references that could not yet be resolved.
#[derive(Debug, Default)]
pub(crate) struct ReferenceCheck {
    defined: Defined,
    /// Every reference not yet defined when its record was read, in source
    /// order. Resolved again at `ENDSEC`, because ISO 10303-21:2016 §11.2
    /// allows an instance to be referenced before it is defined.
    unresolved: Vec<Key>,
    /// One entry per record with unresolved references: the record's span
    /// and the end of its run in `unresolved`.
    pending: Vec<(Span, usize)>,
}

impl ReferenceCheck {
    /// Records one successfully parsed data record spanning `span`.
    pub(crate) fn record(
        &mut self,
        record: &DataRecord<String>,
        span: Span,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if !self.defined.insert(Key::of(&record.id)) {
            diagnostics.push(Diagnostic::duplicate_id(span, record.id.clone()));
        }
        let start = self.unresolved.len();
        for component in &record.records {
            for parameter in &component.parameters {
                self.collect(parameter);
            }
        }
        if self.unresolved.len() > start {
            self.pending.push((span, self.unresolved.len()));
        }
    }

    /// Collects not-yet-defined references, depth-first and left to right,
    /// so they stay in source order. Recursion depth is bounded by the
    /// parser's [`MAX_PARAMETER_NESTING`](crate::MAX_PARAMETER_NESTING).
    fn collect(&mut self, parameter: &Parameter) {
        match parameter {
            Parameter::Ref(id) => {
                let key = Key::of(id);
                if !self.defined.contains(&key) {
                    self.unresolved.push(key);
                }
            }
            Parameter::List(items) => {
                for item in items {
                    self.collect(item);
                }
            }
            Parameter::Typed { value, .. } => self.collect(value),
            _ => {}
        }
    }

    /// Resolves forward references at the end of `DATA`, reports each
    /// distinct id that is still missing once per record, then orders all
    /// diagnostics by source position.
    pub(crate) fn finish(self, diagnostics: &mut Vec<Diagnostic>) {
        let mut reported: HashSet<&Key> = HashSet::new();
        let mut start = 0;
        for (span, end) in &self.pending {
            reported.clear();
            for key in &self.unresolved[start..*end] {
                if !self.defined.contains(key) && reported.insert(key) {
                    diagnostics.push(Diagnostic::dangling_reference(*span, key.to_id()));
                }
            }
            start = *end;
        }
        // Stable: diagnostics on the same record keep their emission order
        // (duplicate first, then its dangling references in source order).
        diagnostics.sort_by_key(|diagnostic| diagnostic.span().start);
    }
}
