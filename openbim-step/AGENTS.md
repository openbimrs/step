# openbim-step

Generic ISO 10303-21 physical-file and ISO 10303-11 EXPRESS language infrastructure.

## Owns

- tokens, source spans, syntax diagnostics, and complete classic STEP string escaping;
- arbitrary-precision instance IDs, records, parameters, headers, and exchange sections;
- parser, deterministic writer, record partitioning, and incremental event sinks;
- schema-neutral EXPRESS declarations, type expressions, and parser diagnostics.

## Does not own

- IFC headers/schema-version policy;
- IFC model/value conversion or entity graph construction;
- IFC registries, inherited layouts, domain/select resolution, validation, migration, or inference.

See sibling `PLAN.md` for active work.
