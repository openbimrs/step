# STEP workspace

Canonical repository for generic ISO 10303 infrastructure.

## Children

- `openbim-step/` — Part 21 syntax/model/parser/writer and generic EXPRESS parser/AST.
- `scripts/` — standalone verification gate.
- `.github/workflows/` — CI.

## Boundary

This workspace must not depend on IFC or any application schema. Generic Part 21 parsing works without an EXPRESS schema; schema-aware integration is optional and layered above syntax. Active extraction state is tracked in `PLAN.md`.
