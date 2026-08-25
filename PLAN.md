# STEP extraction plan

Status: active
Last updated: 2026-08-25

## Goal

Extract generic ISO 10303-21 and ISO 10303-11 mechanics from `openbimrs/ifc` without moving IFC policy downward.

## Planned file map

- `openbim-step/src/{diagnostic,lexer,escape,model,header,parser,writer,partition}.rs` — Part 21 infrastructure.
- `openbim-step/src/express.rs` — generic EXPRESS declarations and parser/AST.
- `openbim-step/tests/` — syntax, round-trip, diagnostics, events, EXPRESS, and architecture regressions.
- `scripts/gate.sh` — standalone package/format/test/lint/doc checks.

## Acceptance

- [ ] RED tests prove the crate is absent before extraction.
- [ ] No IFC dependency or source import.
- [ ] Generic Part 21 round-trip and event API pass.
- [ ] Generic EXPRESS fixtures pass.
- [ ] Standalone gate passes.
- [ ] IFC consumes the versioned crate and parent pins the exact STEP repository commit.
