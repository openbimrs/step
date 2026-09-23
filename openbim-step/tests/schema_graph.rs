#![allow(missing_docs)]
//! `SchemaGraph` over multiple inheritance and explicit redeclarations.

use openbim_step::SchemaGraph;

// ---- #2 / #3 against SchemaGraph -----------------------------------------

/// Diamond: `c SUBTYPE OF (a, b)`, both under `top`. §12.2.5.2 gives
/// `t` once (first reference wins), then `a`, then `b`, then `c`.
const DIAMOND: &str = "\
SCHEMA D;
ENTITY top; t : INTEGER; END_ENTITY;
ENTITY a SUBTYPE OF (top); x : INTEGER; END_ENTITY;
ENTITY b SUBTYPE OF (top); y : INTEGER; END_ENTITY;
ENTITY c SUBTYPE OF (a, b); z : INTEGER; END_ENTITY;
ENTITY d SUBTYPE OF (c); w : INTEGER; END_ENTITY;
END_SCHEMA;";

#[test]
fn multiple_supertypes_are_laid_out_in_subtype_of_order_once_each() {
    let g = SchemaGraph::from_express(DIAMOND);
    assert_eq!(g.attribute_names("c"), ["t", "x", "y", "z"]);
    assert_eq!(g.attribute_names("d"), ["t", "x", "y", "z", "w"]);
    assert_eq!(g.direct_supertypes("c"), ["a", "b"]);
    assert_eq!(g.supertypes("d"), ["c", "a", "top", "b"]);
    assert!(g.is_a("c", "b"));
    assert!(g.is_a("d", "top"));
    assert!(!g.is_a("a", "b"));
    assert_eq!(g.direct_subtypes("b"), ["c"]);
    assert_eq!(g.subtypes("top"), ["a", "c", "d", "b"]);
}

/// Order is the SUBTYPE OF order, not alphabetical and not declaration order.
#[test]
fn supertype_order_follows_the_subtype_of_clause() {
    let g = SchemaGraph::from_express(
        "SCHEMA O; ENTITY zz; z : INTEGER; END_ENTITY; ENTITY aa; a : INTEGER; END_ENTITY;
         ENTITY both SUBTYPE OF (zz, aa); END_ENTITY; END_SCHEMA;",
    );
    assert_eq!(g.attribute_names("both"), ["z", "a"]);
}

#[test]
fn explicit_redeclaration_keeps_the_inherited_slot() {
    let g = SchemaGraph::from_express(
        r"SCHEMA R;
        ENTITY styled; name : STRING; target : thing; END_ENTITY;
        ENTITY plane SUBTYPE OF (styled);
          SELF\styled.target : plane_target;
          extra : INTEGER;
        END_ENTITY;
        END_SCHEMA;",
    );
    assert_eq!(g.attribute_names("plane"), ["name", "target", "extra"]);
}

/// A cyclic multi-parent schema is illegal EXPRESS but must terminate, and
/// promptly.
///
/// The depth bound alone does not make this safe: with two parents per level
/// an unmemoized walk has 2^depth paths, so it "terminates" only in theory.
/// The visited set is what bounds the work. The walk runs on a worker thread
/// so a regression fails this test instead of hanging the suite.
#[test]
fn cyclic_multiple_inheritance_terminates() {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let g = SchemaGraph::from_express(
            "SCHEMA X; ENTITY p SUBTYPE OF (q, p); a : INTEGER; END_ENTITY;
             ENTITY q SUBTYPE OF (p); b : INTEGER; END_ENTITY; END_SCHEMA;",
        );
        let layout = g.attribute_names("p").len();
        let _ = g.supertypes("p");
        let _ = g.subtypes("p");
        let _ = done.send((layout, g.is_a("p", "q")));
    });
    let (layout, is_a) = finished
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("cyclic multiple inheritance must not blow up the walk");
    assert_eq!(layout, 2, "each entity contributes its slots once");
    assert!(is_a);
}

/// `subtypes` must stay the exact inverse of `is_a` once an entity has several
/// parents: the downward index is built from every declared parent.
#[test]
fn subtypes_is_the_inverse_of_is_a_under_multiple_inheritance() {
    let g = SchemaGraph::from_express(DIAMOND);
    let names: Vec<&str> = g.entity_names().collect();
    for &ancestor in &names {
        let down = g.subtypes(ancestor);
        for &candidate in &names {
            let expected = candidate != ancestor && g.is_a(candidate, ancestor);
            assert_eq!(
                down.contains(&candidate),
                expected,
                "{candidate} vs {ancestor}"
            );
        }
    }
}

/// Real AP schemas, when a local copy is configured (`STEP_AP_SCHEMA_DIR`
/// containing `242_mim_lf.exp`, e.g. `data/ap242` of the `stepcode` repository).
///
/// Slot counts are cross-checked against OCCT's generated readers
/// (`CheckNbParams` in `RWStep*`), which agree with these on 523 of 525 AP242
/// entities. The two exceptions are OCCT deviating from ISO 10303-21:2016:
/// - `common_datum`: OCCT reads `shape_aspect` twice (9 slots) through the
///   `composite_shape_aspect`/`datum` diamond; §12.2.5.2 says a supertype
///   reached again is ignored, giving 5.
/// - `characterized_representation`: OCCT drops the two derived
///   redeclarations of `characterized_object` (4); §12.2.6 keeps them as `*`
///   slots, giving 5.
#[test]
fn ap242_positional_layouts_match_part21_on_multiple_inheritance() {
    let Some(dir) = std::env::var_os("STEP_AP_SCHEMA_DIR") else {
        return;
    };
    let path = std::path::Path::new(&dir).join("242_mim_lf.exp");
    let g = SchemaGraph::from_express(&std::fs::read_to_string(path).expect("schema readable"));
    assert_eq!(
        g.attribute_names("advanced_face"),
        ["name", "bounds", "face_geometry", "same_sense"]
    );
    assert_eq!(
        g.attribute_names("annotation_plane"),
        ["name", "styles", "item", "elements"]
    );
    assert_eq!(
        g.attribute_names("common_datum"),
        [
            "name",
            "description",
            "of_shape",
            "product_definitional",
            "identification"
        ]
    );
    assert_eq!(
        g.attribute_names("characterized_representation"),
        ["name", "items", "context_of_items", "name", "description"]
    );
    assert!(g.is_a("common_datum", "datum"));
}
