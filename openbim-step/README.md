# openbim-step

`openbim-step` is the reusable, IFC-independent Rust substrate for ISO 10303 STEP syntax.

Implemented capabilities:

- ISO 10303-21 lexical tokens, spans, diagnostics, parsing, deterministic writing, record-aligned partitioning, and streaming event/sink parsing;
- schema-independent HEADER and single-DATA exchange models with simple and complex instances;
- lossless lexical storage for unbounded instance identifiers and numeric values;
- classic and Unicode STEP string forms, including alphabet-selection directives and edition-3 direct UTF-8;
- structural, explicitly partial ISO 10303-11 EXPRESS parsing and AST types.

IFC graph conversion, schema lowering, validation, migration, and domain policy deliberately live in the downstream [`openbimrs/ifc`](https://github.com/openbimrs/ifc) family.

See the [repository README](https://github.com/openbimrs/step#readme) for scope, examples, and support boundaries.
