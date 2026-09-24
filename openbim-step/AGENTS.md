# openbim-step

Generic ISO 10303-21 physical-file and ISO 10303-11 EXPRESS language infrastructure.

## Owns

- tokens, source spans, syntax diagnostics, and complete classic STEP string escaping;
- arbitrary-precision instance IDs, records, parameters, headers, and exchange sections;
- parser, deterministic writer, record partitioning, and incremental event sinks;
- opt-in Part 21 reference integrity (duplicate ids, dangling references) as
  non-fatal diagnostics, in `src/references.rs` — syntax-level only, never
  schema-aware;
- schema-neutral EXPRESS declarations, type expressions, and parser diagnostics;
- the schema graph over those declarations: supertype graphs (multiple
  inheritance), Part 21 positional attribute order, and defined-type alias
  resolution. Slot counts are cross-checked against OCCT in
  `tests/schema_graph.rs` when `STEP_AP_SCHEMA_DIR` points at AP schemas.

## Does not own

- application-schema registries, bundled schema artifacts, or schema-version policy;
- application model/value conversion or entity graph construction;
- domain/select resolution, validation, migration, or inference.

See sibling `PLAN.md` for active work.
