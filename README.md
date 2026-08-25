# openbim-step

Pure-Rust, schema-independent infrastructure for ISO 10303.

## Implemented scope

- ISO 10303-21 tokens with source spans and diagnostics;
- STEP string escaping, including legacy `\P` alphabet selection for `\S` escapes;
- generic HEADER records, one DATA section, arbitrary-precision instance IDs, simple and complex records, user-defined keywords, and parameters;
- arbitrary-precision integer/real lexemes, validated binary syntax, and deterministic syntax-safe writing;
- owning parser, record partitioning, and incrementally emitted event/sink parsing;
- structural EXPRESS declaration parsing and generic AST.

The current Part 21 model covers the classic exchange structure used by IFC.
Edition 3 anchor/reference sections and multiple DATA sections are not yet
represented. The EXPRESS frontend is intentionally incomplete: complete
functions, procedures, rules, WHERE/UNIQUE semantics, and executable schema
validation remain future work. IFC-specific graph mapping and policy live in
[`openbimrs/ifc`](https://github.com/openbimrs/ifc).

```bash
bash scripts/gate.sh
```

MIT licensed.
