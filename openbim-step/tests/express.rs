#![allow(missing_docs)]

use openbim_step::express::{parse, Attribute, EntityDef, ParsedSchema, TypeDef, TypeKind};

#[test]
fn schema_model_builders_preserve_the_ifc_schema_surface() {
    let attribute = Attribute::new("Items", "IfcLabel").optional().aggregate();
    assert!(attribute.optional);
    assert!(attribute.aggregate);

    let entity = EntityDef::new("IfcExample")
        .with_supertype("IfcRoot")
        .with_attribute(attribute);
    assert_eq!(entity.supertype.as_deref(), Some("IfcRoot"));
    assert_eq!(entity.attributes.len(), 1);

    let defined = TypeDef {
        name: "IfcLabel".into(),
        kind: TypeKind::Defined("STRING".into()),
    };
    assert!(defined.is_defined());
}

const SCHEMA: &str = r"
SCHEMA DEMO;
(* ENTITY Fake; value : TEXT; END_ENTITY; *)
TYPE Distance = REAL;
END_TYPE;
TYPE Shade = ENUMERATION OF (RED, GREEN, BLUE);
END_TYPE;
TYPE AnyValue = SELECT (Distance, Shade);
END_TYPE;
ENTITY Root ABSTRACT SUPERTYPE OF (ONEOF(Item));
  Label : OPTIONAL STRING;
END_ENTITY;
ENTITY Item SUBTYPE OF (Root);
  Size : Distance;
  Points : LIST [1:?] OF Distance;
DERIVE
  Doubled : Distance := Size * 2;
WHERE
  Positive : Size > 0;
END_ENTITY;
END_SCHEMA;
";

#[test]
fn structural_partial_express_parser_extracts_supported_declarations() {
    let ParsedSchema {
        name,
        entities,
        types,
    } = parse(SCHEMA);
    assert_eq!(name, "DEMO");
    assert_eq!(entities.len(), 2);
    assert_eq!(types.len(), 3);

    let root = &entities[0];
    assert_eq!(
        root,
        &EntityDef {
            name: "Root".into(),
            supertype: None,
            abstract_: true,
            attributes: vec![Attribute {
                name: "Label".into(),
                type_name: "STRING".into(),
                optional: true,
                aggregate: false
            }],
            derived: Vec::new(),
        }
    );
    let item = &entities[1];
    assert_eq!(item.supertype.as_deref(), Some("Root"));
    assert_eq!(
        item.attributes.len(),
        2,
        "DERIVE and WHERE are not explicit slots"
    );
    assert!(item.attributes[1].aggregate);

    assert_eq!(
        types[0],
        TypeDef {
            name: "Distance".into(),
            kind: TypeKind::Defined("REAL".into())
        }
    );
    assert_eq!(
        types[1].kind,
        TypeKind::Enumeration(vec!["RED".into(), "GREEN".into(), "BLUE".into()])
    );
    assert_eq!(
        types[2].kind,
        TypeKind::Select(vec!["Distance".into(), "Shade".into()])
    );
}

/// A subtype may redeclare an inherited attribute as DERIVED. Part 21 writes
/// such a slot as `*`, which is neither a value nor `$`, so a writer that does
/// not know the attribute is derived cannot produce a conforming file.
#[test]
fn derive_blocks_report_redeclared_attribute_names() {
    let source = "\
SCHEMA test;
ENTITY parent;
  Precision : REAL;
  Dimension : INTEGER;
END_ENTITY;
ENTITY child
 SUBTYPE OF (parent);
  ParentRef : parent;
 DERIVE
  SELF\\parent.Precision : REAL := NVL(ParentRef.Precision, 1.E-5);
  SELF\\parent.Dimension : INTEGER := ParentRef.Dimension;
 WHERE
  NoSub : TRUE;
END_ENTITY;
END_SCHEMA;
";
    let schema = parse(source);
    let child = schema
        .entities
        .iter()
        .find(|entity| entity.name == "child")
        .expect("child entity");

    assert_eq!(
        child.derived,
        vec!["Precision".to_owned(), "Dimension".to_owned()],
        "the SELF\\Entity. prefix names the supertype, not the attribute"
    );
    assert!(
        child.is_derived("precision"),
        "matching is case-insensitive"
    );
    assert!(
        !child.is_derived("ParentRef"),
        "explicit attributes are not derived"
    );

    // The WHERE clause must not leak into the derived list.
    assert!(!child.is_derived("NoSub"));

    // An entity without a DERIVE block reports none.
    let parent = schema
        .entities
        .iter()
        .find(|entity| entity.name == "parent")
        .expect("parent entity");
    assert!(parent.derived.is_empty());
}

/// A derived attribute that is not a redeclaration has no qualifying prefix.
#[test]
fn unqualified_derived_attributes_are_reported() {
    let source = "\
SCHEMA test;
ENTITY thing;
  Length : REAL;
 DERIVE
  Area : REAL := Length * Length;
END_ENTITY;
END_SCHEMA;
";
    let schema = parse(source);
    let thing = &schema.entities[0];
    assert_eq!(thing.derived, vec!["Area".to_owned()]);
    assert_eq!(
        thing.attributes.len(),
        1,
        "a derived attribute is not an explicit positional attribute"
    );
}

/// The DERIVE block must end where the next clause begins.
///
/// A single WHERE rule cannot prove this: its statement still carries the
/// `WHERE` keyword, so it fails the identifier check by accident. From the
/// second rule onward the keyword is gone and a bad boundary silently reports
/// rule labels as derived attributes. Real schemas routinely have several.
#[test]
fn where_rule_labels_are_not_reported_as_derived() {
    let source = "\
SCHEMA test;
ENTITY child;
  ParentRef : INTEGER;
 DERIVE
  SELF\\parent.Precision : REAL := 1.0;
 WHERE
  FirstRule : TRUE;
  SecondRule : TRUE;
  ThirdRule : TRUE;
END_ENTITY;
END_SCHEMA;
";
    let schema = parse(source);
    let child = &schema.entities[0];
    assert_eq!(
        child.derived,
        vec!["Precision".to_owned()],
        "only the DERIVE statement, not the WHERE rule labels"
    );
    for rule in ["FirstRule", "SecondRule", "ThirdRule"] {
        assert!(
            !child.is_derived(rule),
            "{rule} is a constraint, not an attribute"
        );
    }
}

/// The same boundary, for the other clauses that can follow DERIVE.
#[test]
fn inverse_and_unique_clauses_do_not_leak_into_derived() {
    let source = "\
SCHEMA test;
ENTITY child;
  Ref : INTEGER;
 DERIVE
  Computed : REAL := 1.0;
 INVERSE
  FirstBack : SET OF other FOR Ref;
  SecondBack : SET OF other FOR Ref;
 UNIQUE
  FirstKey : Ref;
  SecondKey : Ref;
END_ENTITY;
END_SCHEMA;
";
    let schema = parse(source);
    let child = &schema.entities[0];
    assert_eq!(child.derived, vec!["Computed".to_owned()]);
    for name in ["FirstBack", "SecondBack", "FirstKey", "SecondKey"] {
        assert!(!child.is_derived(name), "{name} must not be derived");
    }
}

/// `UNIQUE` inside an aggregate declaration does not end the attribute list.
///
/// EXPRESS reuses block keywords as declaration modifiers: `LIST [1:?] OF
/// UNIQUE X` is an attribute, not a UNIQUE block. Ending the body at the first
/// occurrence truncated the entity, and every attribute after it vanished.
/// `IfcTypeProduct` lost `RepresentationMaps` and `Tag` this way, which made
/// every IFC product type impossible to author.
#[test]
fn a_unique_aggregate_does_not_truncate_the_attribute_list() {
    let source = "\
ENTITY Holder
 SUPERTYPE OF (ONEOF
    (SubA
    ,SubB))
 SUBTYPE OF (Base);
\tMaps : OPTIONAL LIST [1:?] OF UNIQUE Target;
\tTag : OPTIONAL Label;
 INVERSE
\tUsedBy : SET [0:?] OF Other FOR Thing;
 WHERE
\tRule : EXISTS(Tag);
END_ENTITY;
";
    let schema = parse(source);
    let holder = schema
        .entities
        .iter()
        .find(|entity| entity.name == "Holder")
        .expect("Holder parsed");

    let names: Vec<&str> = holder
        .attributes
        .iter()
        .map(|attribute| attribute.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["Maps", "Tag"],
        "the attribute after the inline UNIQUE must survive"
    );
    assert_eq!(holder.supertype.as_deref(), Some("Base"));
}

/// A real `UNIQUE` block still ends the attribute list.
///
/// The fix must not swing the other way and swallow genuine blocks.
#[test]
fn a_statement_level_unique_block_still_ends_the_attributes() {
    let source = "\
ENTITY Thing;
\tName : Label;
 UNIQUE
\tOnlyOne : Name;
END_ENTITY;
";
    let schema = parse(source);
    let thing = schema
        .entities
        .iter()
        .find(|entity| entity.name == "Thing")
        .expect("Thing parsed");
    let names: Vec<&str> = thing
        .attributes
        .iter()
        .map(|attribute| attribute.name.as_str())
        .collect();
    assert_eq!(names, ["Name"], "the UNIQUE block is not an attribute");
}
