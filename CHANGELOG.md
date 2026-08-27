# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.1] - 2026-08-27

### Fixed

- Recovery no longer consumes `ENDSEC` when the failing record is the last in
  `DATA`. The diagnostic span for a bare instance id covers the following
  token, so resynchronizing past the span swallowed the section terminator and
  failed the whole file at `END-ISO-10303-21`. Recovery now rescans from just
  after the damaged record's first byte.

## [0.3.0] - 2026-08-27

### Added

- Opt-in malformed-record recovery: `ParseOptions`, `OnMalformed`,
  `parse_with`, and `parse_events_with`. Under `OnMalformed::Skip` an
  unparsable data record is reported as a `Diagnostic` and the parser
  resynchronizes on the next record, so a consumer can load a damaged export
  and still show exactly what was dropped. Parsing stays strict by default,
  and header structure, section structure, and the physical-file marker remain
  fatal under every policy.
- `ParseOutcome`, carrying the parsed exchange together with the non-fatal
  `Diagnostic` list in source order.
- `EntityDef::derived`: the attribute names declared in an entity's `DERIVE`
  block, with `EntityDef::with_derived` and `EntityDef::is_derived`. A subtype
  may redeclare an inherited explicit attribute as derived; Part 21 writes such
  a slot as `*`, which is neither a value nor `$`. Without this a writer cannot
  tell the three apart and cannot produce a conforming file. Names are reported
  unqualified -- the `SELF\\Entity.` prefix names the declaring supertype, not
  the attribute. Initialiser expressions are still not evaluated.

### Changed

- **Breaking:** `EntityDef` gained a public field, so struct-literal
  construction must add `derived`. Builder-style construction is unaffected.

## [0.2.1] - 2026-08-25

### Fixed

- Accept low lines wherever Part 21's `UPPER` production permits them, including
  schema/user-defined keywords and enumeration values.
- Ignore `\\N\\` and `\\F\\` print directives inside wide-string payloads,
  between doubled apostrophes, and before the physical-file marker, while
  preserving escaped literal `\\N\\`/`\\F\\` text.

## [0.2.0] - 2026-08-25

### Changed

- Token byte payloads now use `Cow<[u8]>`, preserving allocation-free normal tokens while allowing line delimiters inside tokens to be removed correctly.

### Fixed

- Enforced mandatory Part 21 header records exactly once and in order.
- Ignored line delimiters throughout lexical tokens while retaining original source spans.
- Preserved malformed wide escapes instead of replacing or discarding their bytes.
- Added `\\N\\`/`\\F\\` print-directive handling between tokens and inside strings/binaries, plus standards-compliant `\\X4\\` output for supplementary Unicode.
- Rejected user-defined keywords in dotted enumeration values.

## [0.1.0] - 2026-08-25

### Added

- Generic ISO 10303-21 tokens, spans, diagnostics, escaping, syntax model, parser, writer, record partitioning, and event/sink parsing.
- Schema-independent structural ISO 10303-11 EXPRESS parser and AST.
- Standalone architecture, round-trip, diagnostics, partition, event, and external-corpus gates.
- Arbitrary-precision instance IDs, classic user-defined keywords, and legacy alphabet-selection decoding.
- Incremental event parsing with bounded token buffering.

[Unreleased]: https://github.com/openbimrs/step/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/openbimrs/step/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/openbimrs/step/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/openbimrs/step/releases/tag/v0.1.0
