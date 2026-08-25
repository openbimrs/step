# openbim-step implementation plan

Status: active extraction
Last updated: 2026-08-25

## Planned file map

- `src/diagnostic.rs` — source locations, spans, and errors.
- `src/lexer.rs` / `src/escape.rs` — Part 21 lexical mechanics.
- `src/model.rs` / `src/header.rs` — generic syntax model and typed standard headers.
- `src/parser.rs` / `src/writer.rs` — owning and event parsing plus deterministic writing.
- `src/partition.rs` — record-aligned partition discovery.
- `src/express.rs` — generic EXPRESS language model/parser.
- `tests/` — executable behavior and dependency boundaries.

## Work

- [ ] Establish RED tests.
- [ ] Extract and rename generic implementation.
- [ ] Verify no IFC dependency.
- [ ] Run standalone and consumer integration gates.
