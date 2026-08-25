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
