# openbimrs/step

Pure-Rust, schema-independent ISO 10303 infrastructure.

- `openbim-step/` owns ISO 10303-21 lexical/syntax parsing and writing plus the reusable ISO 10303-11 EXPRESS language layer.
- It must not depend on IFC or any other application schema.
- Schema-specific lowering, validation, version policy, and graph construction stay in consumer repositories.
- Run `bash scripts/gate.sh` before landing changes.

Read `AGENTS.md` at the repository root and the nested file in the crate before editing.
