#![allow(missing_docs)]
//! Complex entity instances resolved against `SchemaGraph` (#5).
//!
//! ISO 10303-21:2016 §12.2.5.3 "External mapping": a complex instance is one
//! `SIMPLE_RECORD` per partial entity value, in ascending order of entity
//! name, and each record carries only the explicit attributes *its own*
//! entity declares -- not the inherited ones. Every supertype of every part
//! must appear too (NOTE 2), even one with no attributes.

use openbim_step::schema::{ComplexIssue, ComplexLayout};
use openbim_step::{parse, SchemaGraph};

/// The shape of the AP214 rational B-spline curve and SI unit structures,
/// written from scratch: a multi-level tree where two subtypes of one
/// supertype combine, one part adds no attributes, and one part redeclares
/// an inherited attribute as derived.
const GEOMETRY: &str = "\
SCHEMA G;
ENTITY item; name : STRING; END_ENTITY;
ENTITY geo SUBTYPE OF (item); END_ENTITY;
ENTITY crv SUBTYPE OF (geo); END_ENTITY;
ENTITY bounded SUBTYPE OF (crv); END_ENTITY;
ENTITY spline SUBTYPE OF (bounded);
  degree : INTEGER;
  points : LIST [2:?] OF item;
END_ENTITY;
ENTITY spline_with_knots SUBTYPE OF (spline);
  knots : LIST [2:?] OF REAL;
END_ENTITY;
ENTITY rational_spline SUBTYPE OF (spline);
  weights : LIST [2:?] OF REAL;
END_ENTITY;
ENTITY unit; dims : INTEGER; END_ENTITY;
ENTITY length_unit SUBTYPE OF (unit); END_ENTITY;
ENTITY si_unit SUBTYPE OF (unit);
  prefix : OPTIONAL INTEGER;
  kind : INTEGER;
DERIVE
  SELF\\unit.dims : INTEGER := 0;
END_ENTITY;
END_SCHEMA;";

fn graph() -> SchemaGraph {
    SchemaGraph::from_express(GEOMETRY)
}

fn slot_names(layout: &ComplexLayout<'_>) -> Vec<(String, Vec<String>)> {
    layout
        .parts()
        .iter()
        .map(|part| {
            (
                part.name().to_owned(),
                part.slots()
                    .iter()
                    .map(|slot| slot.attribute().name.clone())
                    .collect(),
            )
        })
        .collect()
}

fn owned(pairs: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    pairs
        .iter()
        .map(|(name, slots)| {
            (
                (*name).to_owned(),
                slots.iter().map(|slot| (*slot).to_owned()).collect(),
            )
        })
        .collect()
}

/// Each part gets only its own explicit attributes, in declaration order.
#[test]
fn each_part_carries_only_its_own_explicit_attributes() {
    let g = graph();
    let layout = g.resolve_complex(&[
        "BOUNDED",
        "CRV",
        "GEO",
        "ITEM",
        "RATIONAL_SPLINE",
        "SPLINE",
        "SPLINE_WITH_KNOTS",
    ]);
    assert!(layout.is_valid(), "{:?}", layout.issues());
    assert_eq!(
        slot_names(&layout),
        owned(&[
            ("BOUNDED", &[]),
            ("CRV", &[]),
            ("GEO", &[]),
            ("ITEM", &["name"]),
            ("RATIONAL_SPLINE", &["weights"]),
            ("SPLINE", &["degree", "points"]),
            ("SPLINE_WITH_KNOTS", &["knots"]),
        ])
    );
}

/// `attribute_names` answers the internal mapping (inherited slots first).
/// That is the wrong shape for a partial record, which is why a separate
/// query exists.
#[test]
fn a_partial_record_is_not_the_internal_mapping() {
    let g = graph();
    assert_eq!(
        g.attribute_names("rational_spline"),
        ["name", "degree", "points", "weights"]
    );
    let layout = g.resolve_complex(&["BOUNDED", "CRV", "GEO", "ITEM", "RATIONAL_SPLINE", "SPLINE"]);
    assert_eq!(layout.parts()[4].slots().len(), 1);
}

/// The unified type set: every part and every supertype, each once.
#[test]
fn types_are_the_supertype_closure_in_name_order() {
    let g = graph();
    let layout = g.resolve_complex(&["LENGTH_UNIT", "SI_UNIT", "UNIT"]);
    assert!(layout.is_valid(), "{:?}", layout.issues());
    assert_eq!(layout.types(), ["length_unit", "si_unit", "unit"]);
}

/// A slot some other part redeclares as derived is written `*` in a
/// conforming file (as `NAMED_UNIT(*)` is in real SI unit instances).
#[test]
fn a_slot_redeclared_as_derived_by_another_part_is_marked() {
    let g = graph();
    let layout = g.resolve_complex(&["LENGTH_UNIT", "SI_UNIT", "UNIT"]);
    let unit = &layout.parts()[2];
    assert_eq!(unit.slots()[0].attribute().name, "dims");
    assert!(unit.slots()[0].is_derived());
    let si = &layout.parts()[1];
    assert!(si.slots().iter().all(|slot| !slot.is_derived()));

    // Without the redeclaring part in the set, the slot is an ordinary one.
    let alone = g.resolve_complex(&["LENGTH_UNIT", "UNIT"]);
    assert!(!alone.parts()[1].slots()[0].is_derived());
}

#[test]
fn an_unknown_part_is_reported_and_the_rest_still_resolve() {
    let g = graph();
    let layout = g.resolve_complex(&["LENGTH_UNIT", "MYSTERY", "UNIT"]);
    assert_eq!(layout.issues(), [ComplexIssue::UnknownType { index: 1 }]);
    assert!(layout.parts()[1].entity().is_none());
    assert!(layout.parts()[1].slots().is_empty());
    assert_eq!(layout.parts()[2].slots().len(), 1);
    assert!(!layout.is_valid());
}

/// §12.2.5.3: ascending order of entity names, octets of the upper-case
/// form. `_` (0x5F) sorts after the letters, so `BOUNDED` < `B_X`.
#[test]
fn parts_out_of_name_order_are_reported() {
    let g = graph();
    let layout = g.resolve_complex(&["UNIT", "LENGTH_UNIT"]);
    assert_eq!(layout.issues(), [ComplexIssue::OutOfOrder { index: 1 }]);
    // Parts keep the order they were given in; nothing is re-sorted.
    assert_eq!(layout.parts()[0].name(), "UNIT");
}

#[test]
fn ordering_is_by_octet_so_underscore_sorts_after_letters() {
    let schema = SchemaGraph::from_express(
        "SCHEMA O; ENTITY b_x; END_ENTITY; ENTITY bounded; END_ENTITY; \
         ENTITY both SUBTYPE OF (b_x, bounded); END_ENTITY; END_SCHEMA;",
    );
    assert!(schema
        .resolve_complex(&["BOTH", "BOUNDED", "B_X"])
        .is_valid());
    let wrong = schema.resolve_complex(&["BOTH", "B_X", "BOUNDED"]);
    assert_eq!(wrong.issues(), [ComplexIssue::OutOfOrder { index: 2 }]);
}

/// Names are case-insensitive, so `unit` and `UNIT` are the same part.
#[test]
fn a_duplicate_part_is_reported() {
    let g = graph();
    let layout = g.resolve_complex(&["LENGTH_UNIT", "UNIT", "unit"]);
    assert_eq!(layout.issues(), [ComplexIssue::Duplicate { index: 2 }]);
}

/// NOTE 2 of §12.2.5.3: every partial value is required, including a
/// supertype with no explicit attributes.
#[test]
fn a_missing_supertype_is_reported_by_name() {
    let g = graph();
    let layout = g.resolve_complex(&["BOUNDED", "ITEM", "RATIONAL_SPLINE", "SPLINE"]);
    assert_eq!(
        layout.issues(),
        [
            ComplexIssue::MissingSupertype {
                name: "crv".to_owned()
            },
            ComplexIssue::MissingSupertype {
                name: "geo".to_owned()
            },
        ]
    );
    // The types still describe the whole instance.
    assert!(layout.types().contains(&"crv"));
}

#[test]
fn issues_combine_and_come_out_in_a_stable_order() {
    let g = graph();
    let layout = g.resolve_complex(&["UNIT", "NOPE", "LENGTH_UNIT", "UNIT"]);
    // `NOPE` is both unknown and, sorting before `UNIT`, out of order: an
    // unknown name is still a name, and its position is still checked.
    assert_eq!(
        layout.issues(),
        [
            ComplexIssue::UnknownType { index: 1 },
            ComplexIssue::OutOfOrder { index: 1 },
            ComplexIssue::OutOfOrder { index: 2 },
            ComplexIssue::Duplicate { index: 3 },
        ]
    );
}

#[test]
fn an_empty_set_is_empty_and_valid() {
    let g = graph();
    let layout = g.resolve_complex(&[]);
    assert!(layout.parts().is_empty());
    assert!(layout.types().is_empty());
    assert!(layout.is_valid());
}

/// The spec's own example (§12.2.5.3 EXAMPLE 1), `#3 = (AA(..)BB(..)CC(..))`.
#[test]
fn the_specification_example_resolves_to_one_slot_per_part() {
    let schema = SchemaGraph::from_express(
        "SCHEMA E; ENTITY aa SUPERTYPE OF (bb ANDOR cc); attrib_a : STRING; END_ENTITY; \
         ENTITY bb SUBTYPE OF (aa); attrib_b : INTEGER; END_ENTITY; \
         ENTITY cc SUBTYPE OF (aa); attrib_c : REAL; END_ENTITY; END_SCHEMA;",
    );
    let exchange = parse(
        b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
          FILE_NAME('','',(''),(''),'','','');\nFILE_SCHEMA(('E'));\nENDSEC;\n\
          DATA;\n#3=(AA('ASTRID')BB(17)CC(4.0));\nENDSEC;\nEND-ISO-10303-21;\n",
    )
    .expect("parse");
    let record = &exchange.data.records[0];
    let names: Vec<&str> = record
        .records
        .iter()
        .map(|part| part.name.as_str())
        .collect();
    let layout = schema.resolve_complex(&names);
    assert!(layout.is_valid(), "{:?}", layout.issues());
    for (part, written) in layout.parts().iter().zip(&record.records) {
        assert_eq!(
            part.slots().len(),
            written.parameters.len(),
            "{}",
            part.name()
        );
    }
    assert_eq!(layout.types(), ["aa", "bb", "cc"]);
}

/// Every complex instance in OCCT's AP214 test files resolves against the
/// AP214 schema: no issues, and each partial record has exactly as many
/// parameters as its entity has explicit attributes. `*` appears only in
/// slots marked derived.
///
/// Needs `STEP_AP_SCHEMA_DIR` (containing `AP214E3_2010.exp`) and
/// `STEP_OCCT_DATA_DIR` (OCCT's `data/step`, with `linkrods.step` and
/// `screw.step`). Skips when either is unset.
#[test]
fn every_complex_instance_in_real_ap214_files_resolves() {
    let (Some(schemas), Some(data)) = (
        std::env::var_os("STEP_AP_SCHEMA_DIR"),
        std::env::var_os("STEP_OCCT_DATA_DIR"),
    ) else {
        return;
    };
    let source = std::fs::read_to_string(std::path::Path::new(&schemas).join("AP214E3_2010.exp"))
        .expect("AP214 schema readable");
    let g = SchemaGraph::from_express(&source);
    let mut resolved = 0;
    for file in ["linkrods.step", "screw.step"] {
        let bytes = std::fs::read(std::path::Path::new(&data).join(file)).expect("data readable");
        let exchange = parse(&bytes).expect("parse");
        for record in exchange.data.records.iter().filter(|r| r.records.len() > 1) {
            let names: Vec<&str> = record
                .records
                .iter()
                .map(|part| part.name.as_str())
                .collect();
            let layout = g.resolve_complex(&names);
            assert!(
                layout.is_valid(),
                "{file} #{}: {:?}",
                record.id.as_str(),
                layout.issues()
            );
            for (part, written) in layout.parts().iter().zip(&record.records) {
                assert_eq!(
                    part.slots().len(),
                    written.parameters.len(),
                    "{file} #{} {}",
                    record.id.as_str(),
                    part.name()
                );
                for (slot, value) in part.slots().iter().zip(&written.parameters) {
                    if matches!(value, openbim_step::Parameter::Derived) {
                        assert!(
                            slot.is_derived(),
                            "{file} #{} {}.{} is `*` but not derived",
                            record.id.as_str(),
                            part.name(),
                            slot.attribute().name
                        );
                    }
                }
            }
            resolved += 1;
        }
    }
    assert_eq!(resolved, 314, "255 in linkrods.step + 59 in screw.step");
}

/// A derivation declared on a supertype the instance is missing still marks
/// the slot: `types` is the whole closure, and the derived-ness of a slot is
/// a property of what the instance *is*, not only of what it writes. Here
/// `metric_si_unit` omits its supertype `si_unit`, which derives `unit.dims`.
#[test]
fn a_derivation_on_a_missing_supertype_still_marks_the_slot() {
    let g = SchemaGraph::from_express(&format!(
        "{}\nENTITY metric_si_unit SUBTYPE OF (si_unit); END_ENTITY; END_SCHEMA;",
        GEOMETRY.trim_end_matches("END_SCHEMA;")
    ));
    let layout = g.resolve_complex(&["METRIC_SI_UNIT", "UNIT"]);
    assert_eq!(
        layout.issues(),
        [ComplexIssue::MissingSupertype {
            name: "si_unit".to_owned()
        }]
    );
    let unit = &layout.parts()[1];
    assert_eq!(unit.slots()[0].attribute().name, "dims");
    assert!(unit.slots()[0].is_derived());
}

/// Every known part gets its entry, even one whose entity declares no
/// attributes (§12.2.5.3 NOTE 2): `CRV()` is a real partial record with an
/// empty parameter list, and `parts()[i]` must line up with record `i`.
#[test]
fn a_known_part_without_attributes_resolves_to_its_entity_and_no_slots() {
    let g = graph();
    let layout = g.resolve_complex(&["BOUNDED", "CRV", "GEO", "ITEM", "SPLINE"]);
    assert!(layout.is_valid(), "{:?}", layout.issues());
    let crv = &layout.parts()[1];
    assert_eq!(crv.name(), "CRV");
    assert_eq!(crv.entity().map(|e| e.name.as_str()), Some("crv"));
    assert!(crv.slots().is_empty());
}
