# Changelog

All notable changes to this project are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0] - 2026-09-24

### Changed

- **Breaking:** `EntityDef::supertype: Option<String>` is replaced by
  `EntityDef::supertypes: Vec<String>`, every direct supertype in
  `SUBTYPE OF` order. `EntityDef::supertype()` returns the first, for
  single-inheritance callers; `with_supertype` now appends. Migration:
  `def.supertype.clone()` becomes `def.supertype().map(str::to_owned)`.
- **Breaking:** `EntityDef` gained public `supertypes` and `redeclared`
  fields, so struct-literal construction must add them.

`cargo semver-checks` against the published 0.5.1 reports exactly these two
breaks (`struct_pub_field_missing`, `constructible_struct_adds_field`) and
182 of 184 checks passing.

### Added

- `EntityDef::redeclared`, `EntityDef::is_redeclared`, and `Redeclaration`:
  explicit `SELF\X.a : T;` redeclarations, kept apart from `attributes`.
- `SchemaGraph::direct_supertypes`.

### Fixed

- Multiple inheritance (#2). Every supertype after the first was dropped, so
  `SchemaGraph::attributes` omitted inherited slots and `is_a` missed
  ancestors. Layouts now follow ISO 10303-21:2016 §12.2.5.2: supertypes in
  `SUBTYPE OF` order, higher supertypes first, and a supertype reached twice
  through a diamond counted once. `supertypes`, `subtypes`, and `is_a` walk
  every parent. AP242 has 248 multi-parent entities; IFC has none.
- Explicit redeclarations no longer add a phantom positional slot (#3).
  `SELF\styled_item.item : plane_or_planar_box;` was parsed as a new
  attribute named `SELF\styled_item.item`; ISO 10303-21:2016 §12.2.8 says it
  has no effect on the encoding. This affected 481 AP242 entities, including
  `advanced_face`.
- A `\S\` page escape followed by an apostrophe no longer ends the string
  literal. `\S\` takes exactly one following `LATIN_CODEPOINT`, which includes
  the apostrophe (ISO 10303-21:2016 §5.2, §6.4.3.1), so `'Stra\S\'e'` is one
  string decoding to `Stra§e`. Previously it failed to lex. The recovery
  resynchronizer applies the same rule, and `\\` is consumed as one escaped
  backslash in both so its second byte cannot open a page escape (#1).

Checked against OCCT's per-entity parameter counts (`CheckNbParams` in its
generated `RWStep*` readers): AP242 mismatches fell from 107 to 2, AP203e2
from 24 to 1. The residuals are OCCT departing from Part 21 (`common_datum`
diamond read twice; `characterized_representation` dropping derived `*`
slots) and are pinned in `tests/schema_graph.rs`.

## [0.5.1] - 2026-09-22

### Added

- `SchemaGraph::direct_subtypes` and `SchemaGraph::subtypes`: the downward
  counterpart of `supertypes`. `subtypes(x)` is every entity `y != x` for
  which `is_a(y, x)` holds, at any depth, in a deterministic sorted
  pre-order. A test checks that equivalence for every ordered entity pair.
  Answering "every `IfcElement`" needs this; `EntityDef` records only the
  upward edge, so the child index is built once at construction.

## [0.5.0] - 2026-09-05

### Added

- `EntityDef::where_rules` captures each entity's `WHERE` rules as a label
  and its expression text. Rule expressions are kept as written, with
  whitespace normalised, rather than parsed: recording that a constraint
  exists is separable from evaluating EXPRESS. This lets a consumer prove a
  claim such as "no rule constrains this attribute" instead of asserting it
  from the specification prose. Parsed from the full IFC4X3 schema: 752
  rules across 487 entities.

### Changed

- Relicensed repository-authored work from MIT to `AGPL-3.0-or-later`; historical releases remain under their published MIT terms, and third-party material retains its own terms.

## [0.4.0] - 2026-08-30

### Added

- `schema::SchemaGraph`: the parsed schema as a queryable graph. Supertype
  chains, Part 21 positional attribute order (inherited slots first),
  case-insensitive entity/type lookup, and defined-type alias resolution.
  Previously each application-schema crate reimplemented this; none of it is
  specific to any one schema.

### Fixed

- A block keyword inside an attribute declaration no longer truncates the
  attribute list. `LIST [1:?] OF UNIQUE X` contains `UNIQUE`, which was read
  as the start of a `UNIQUE` block, silently dropping every attribute after
  it. Block keywords are now recognized only at statement level. In IFC4 this
  affected 11 declarations, including `IfcTypeProduct.RepresentationMaps` and
  `.Tag` -- and therefore the positional slots of all 124 entities inheriting
  from it.

## [0.3.2] - 2026-08-27

### Fixed

- Recovery no longer reads record syntax out of a binary literal. `"..."` was
  the one literal kind the resynchronization scan did not track, so a `;`
  inside a blob looked like a record boundary and the bytes after it were
  parsed as real records -- fabricating entities that were never in the source
  and were covered by no diagnostic. An apostrophe inside a blob also inverted
  the scanner state and swallowed the rest of the file. Strings, binaries, and
  comments are now all tracked.

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

[Unreleased]: https://github.com/openbimrs/step/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/openbimrs/step/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/openbimrs/step/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/openbimrs/step/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/openbimrs/step/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/openbimrs/step/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/openbimrs/step/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/openbimrs/step/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/openbimrs/step/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/openbimrs/step/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/openbimrs/step/releases/tag/v0.1.0
