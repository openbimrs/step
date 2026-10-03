# openbim-step

Generic ISO 10303-21 physical-file and ISO 10303-11 EXPRESS language infrastructure.

## Owns

- tokens, source spans, syntax diagnostics, and complete classic STEP string escaping;
- arbitrary-precision instance IDs, records, parameters, headers, and exchange sections;
- the owned model's shape (`Str`, `Instance`, boxed parameter lists). Its
  content must not change with its representation: check any reshaping
  with a canonical content dump against the last release, not `Debug`;
- parser, deterministic writer, record partitioning, and incremental event sinks;
- borrowed events (`parse_events_borrowed`) and the parallel parse
  (`src/parallel.rs`). Invariant for the parallel parse: its result equals
  `parse_with`'s exactly; a slice is used only if its parser lands on the
  slice's end offset, anything else falls back to the sequential parse.
  `tests/parallel.rs` checks equality over traps for the split guess;
- the lazy record index (`src/scan.rs`: `scan`, `decode_record`). Invariant:
  if the scan and the decode of every record succeed, `parse` succeeds with
  exactly those records. Framing uses fast paths, but inter-record bytes go
  through the lexer and a decode must end exactly at its span, so a framing
  slip can only become an error. `tests/scan.rs` checks it on traps and
  3,000 seeded mutants; lexer string changes also need pinned-value tests,
  because they move `parse` and `scan` together;
- performance work is measured, not assumed: equivalence against the last
  release plus instruction counts (the dev VM is shared and noisy);
- opt-in acceptance of reals without a decimal point
  (`ParseOptions::accept_real_without_point`, in `lenient()`): data section
  only, point inserted into the stored value, one warning per number, the
  same in eager, lazy (`decode_record_with`) and parallel parsing
  (`tests/real_without_point.rs`). Its lexer path is `#[cold]` and out of
  line so the common number path stays as measured;
- opt-in Part 21 reference integrity (duplicate ids, dangling references) as
  non-fatal diagnostics, in `src/references.rs` — syntax-level only, never
  schema-aware;
- schema-neutral EXPRESS declarations, type expressions, and parser diagnostics:
  explicit attributes with their aggregation levels and bounds, derived
  names, redeclarations, INVERSE attributes, UNIQUE and WHERE rules (as
  text). The declaration types are `#[non_exhaustive]`: add fields, never
  require struct literals. `tests/express.rs` checks declaration counts and
  known shapes on the IFC2X3/IFC4/IFC4X3 schemas when `STEP_IFC_SCHEMA_DIR`
  points at them (`openbimrs/ifc`'s `references/ifc-spec` layout);
- the schema graph over those declarations: supertype graphs (multiple
  inheritance), Part 21 positional attribute order, and defined-type alias
  resolution. Slot counts are cross-checked against OCCT in
  `tests/schema_graph.rs` when `STEP_AP_SCHEMA_DIR` points at AP schemas.
- complex entity instances (Part 21 external mapping) resolved against the
  schema graph, in `src/schema/complex.rs`. `tests/complex.rs` checks every
  complex instance of OCCT's AP214 test files when `STEP_AP_SCHEMA_DIR` and
  `STEP_OCCT_DATA_DIR` are set.

## Does not own

- application-schema registries, bundled schema artifacts, or schema-version policy;
- application model/value conversion or entity graph construction;
- domain/select resolution, validation, migration, or inference.

See sibling `PLAN.md` for active work.
