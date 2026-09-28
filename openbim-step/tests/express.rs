#![allow(missing_docs)]

use openbim_step::express::{
    parse, AggregateKind, Aggregation, Attribute, Bound, EntityDef, InverseAttribute, ParsedSchema,
    TypeDef, TypeKind, UniqueRule,
};

#[test]
fn schema_model_builders_preserve_the_ifc_schema_surface() {
    let attribute = Attribute::new("Items", "IfcLabel").optional().aggregate();
    assert!(attribute.optional);
    assert!(attribute.aggregate);

    let entity = EntityDef::new("IfcExample")
        .with_supertype("IfcRoot")
        .with_attribute(attribute);
    assert_eq!(entity.supertype(), Some("IfcRoot"));
    assert_eq!(entity.attributes.len(), 1);

    let defined = TypeDef::new("IfcLabel", TypeKind::Defined("STRING".into()));
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
        ..
    } = parse(SCHEMA);
    assert_eq!(name, "DEMO");
    assert_eq!(entities.len(), 2);
    assert_eq!(types.len(), 3);

    let root = &entities[0];
    assert_eq!(
        root,
        &EntityDef::new("Root")
            .abstract_()
            .with_attribute(Attribute::new("Label", "STRING").optional())
    );
    let item = &entities[1];
    assert_eq!(item.supertype(), Some("Root"));
    assert_eq!(
        item.attributes.len(),
        2,
        "DERIVE and WHERE are not explicit slots"
    );
    assert!(item.attributes[1].aggregate);

    assert_eq!(
        types[0],
        TypeDef::new("Distance", TypeKind::Defined("REAL".into()))
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
    assert_eq!(holder.supertype(), Some("Base"));
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

/// A `WHERE` block is captured as labelled rules.
///
/// Recording that a constraint exists, and what it says, is what lets a
/// consumer prove "no rule constrains this" instead of asserting it from
/// prose. The expression is kept verbatim; evaluating EXPRESS is a separate
/// concern.
#[test]
fn where_rules_are_captured_with_labels_and_expressions() {
    let source = "\
ENTITY IfcSurfaceCurve
 SUBTYPE OF (IfcCurve);
\tCurve3D : IfcCurve;
 WHERE
\tCurveIs3D : Curve3D.Dim = 3;
\tCurveIsNotPcurve : NOT ('IFC4.IFCPCURVE' IN TYPEOF(Curve3D));
END_ENTITY;";
    let schema = parse(source);
    let entity = schema
        .entities
        .iter()
        .find(|entity| entity.name.eq_ignore_ascii_case("IfcSurfaceCurve"))
        .expect("entity");
    let labels: Vec<_> = entity
        .where_rules
        .iter()
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(labels, ["CurveIs3D", "CurveIsNotPcurve"]);
    assert_eq!(entity.where_rules[0].expression, "Curve3D.Dim = 3");
    assert_eq!(
        entity.where_rules[1].expression,
        "NOT ('IFC4.IFCPCURVE' IN TYPEOF(Curve3D))"
    );
    // The attribute list must survive the WHERE block.
    assert_eq!(entity.attributes.len(), 1);
}

/// A multi-line rule body is captured whole, not cut at the first newline.
///
/// Rule expressions routinely span lines and nest `QUERY(x <* set | predicate)`;
/// `IfcAdvancedFace` in IFC4X3 declares three such rules, the longest running
/// eleven lines. Whitespace is normalised so the stored text stays comparable.
///
/// The block start uses `find_block_keyword` for the same reason the attribute
/// list does -- a statement-level check rather than a keyword search. No IFC4X3
/// entity currently writes `WHERE` inside an earlier block, so that choice is
/// defence against a legal schema this parser has not met, not a fix for an
/// observed break.
#[test]
fn a_multi_line_query_rule_is_captured_whole() {
    let source = "\
ENTITY Face;
\tBounds : SET OF Bound;
 WHERE
\tFirstRule : SIZEOF(QUERY (b <* Bounds |
\t  NOT ('SCHEMA.LOOP' IN TYPEOF(b)))) = 0;
\tSecondRule : SIZEOF(Bounds) > 0;
END_ENTITY;";
    let schema = parse(source);
    let entity = schema
        .entities
        .iter()
        .find(|entity| entity.name.eq_ignore_ascii_case("Face"))
        .expect("entity");
    let labels: Vec<_> = entity
        .where_rules
        .iter()
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(labels, ["FirstRule", "SecondRule"]);
    // The multi-line rule is normalised to one line, not cut at the newline.
    assert_eq!(
        entity.where_rules[0].expression,
        "SIZEOF(QUERY (b <* Bounds | NOT ('SCHEMA.LOOP' IN TYPEOF(b)))) = 0"
    );
    assert_eq!(entity.attributes.len(), 1);
}

/// `WHERE` appearing inside an earlier block does not start the rule block.
///
/// No IFC4X3 entity does this, but EXPRESS permits it and a keyword search
/// would take the DERIVE-block occurrence as the block start, silently
/// dropping every real rule. This pins the statement-level behaviour.
#[test]
fn a_where_token_inside_an_earlier_block_is_not_the_block_start() {
    let source = "\
ENTITY Holder;
\tItems : SET OF Item;
 DERIVE
\tPicked : Item := QUERY(i <* SELF.Items | i.Kind = 'WHERE')[1];
 WHERE
\tRealRule : SIZEOF(Items) > 0;
END_ENTITY;";
    let schema = parse(source);
    let entity = schema
        .entities
        .iter()
        .find(|entity| entity.name.eq_ignore_ascii_case("Holder"))
        .expect("entity");
    let labels: Vec<_> = entity
        .where_rules
        .iter()
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(labels, ["RealRule"]);
    assert_eq!(entity.derived, ["Picked"]);
}

// ---- #2 multiple inheritance, #3 explicit redeclarations -----------------
// ISO 10303-21:2016 §12.2.5.2: supertypes are processed in SUBTYPE OF order,
// higher supertypes first, and a supertype reached twice counts once.
// §12.2.8: an explicit `SELF\X.a` redeclaration keeps X's slot and adds none.

const MULTI: &str = r"
SCHEMA M;
ENTITY top; t : INTEGER; END_ENTITY;
ENTITY a SUBTYPE OF (top); x : INTEGER; END_ENTITY;
ENTITY b SUBTYPE OF (top); y : INTEGER; END_ENTITY;
ENTITY c SUBTYPE OF (a, b); z : INTEGER; END_ENTITY;
END_SCHEMA;
";

#[test]
fn every_declared_supertype_is_recorded_in_order() {
    let parsed = parse(MULTI);
    let c = parsed.entities.iter().find(|e| e.name == "c").unwrap();
    assert_eq!(c.supertypes, ["a", "b"]);
    assert_eq!(c.supertype(), Some("a"));
    let top = parsed.entities.iter().find(|e| e.name == "top").unwrap();
    assert!(top.supertypes.is_empty());
    assert_eq!(top.supertype(), None);
}

const REDECLARED: &str = r"
SCHEMA R;
ENTITY styled; name : STRING; target : thing; END_ENTITY;
ENTITY plane SUBTYPE OF (styled);
  SELF\styled.target : plane_target;
  extra : INTEGER;
END_ENTITY;
END_SCHEMA;
";

#[test]
fn explicit_redeclarations_are_not_new_attributes() {
    let parsed = parse(REDECLARED);
    let plane = parsed.entities.iter().find(|e| e.name == "plane").unwrap();
    let names: Vec<_> = plane.attributes.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["extra"]);
    assert_eq!(plane.redeclared.len(), 1);
    let r = &plane.redeclared[0];
    assert_eq!(
        (r.supertype.as_str(), r.name.as_str(), r.type_name.as_str()),
        ("styled", "target", "plane_target")
    );
    assert!(plane.is_redeclared("TARGET"));
    assert!(!plane.is_redeclared("extra"));
}

/// Aggregation shapes, INVERSE and UNIQUE blocks, including the traps: a
/// `UNIQUE` inside an attribute's type, lower-case keywords, omitted bounds,
/// expression bounds, and an inverse redeclared by a subtype.
const SHAPES: &str = r"
SCHEMA SHAPES;
ENTITY Owner;
  GlobalId : Label;
  Code : Label;
  Grid : LIST [2:3] OF UNIQUE LIST [1:?] OF Length;
  Maps : list [1:?] of unique Map;
  Tags : OPTIONAL SET OF Label;
  Slots : ARRAY [1:3] OF OPTIONAL Length;
  Knots : LIST [0 : SELF\Owner.Degree] OF Length;
  Pairs : BAG [2:2] OF Length;
  Degree : INTEGER;
DERIVE
  Count : INTEGER := SIZEOF(Maps);
INVERSE
  Parts : SET [0:?] OF Part FOR Whole;
  Host : Part FOR Guest;
  Uses : BAG [1:5] OF Part FOR Part.Used;
UNIQUE
  UR1 : GlobalId;
  UR2 : GlobalId, SELF\Owner.Code;
  Code;
WHERE
  WR1 : SIZEOF(QUERY(m <* Maps | TRUE)) > 0;
END_ENTITY;
ENTITY Special SUBTYPE OF (Owner);
INVERSE
  SELF\Owner.Parts : SET [1:1] OF Part FOR Whole;
END_ENTITY;
END_SCHEMA;
";

fn entity<'a>(schema: &'a ParsedSchema, name: &str) -> &'a EntityDef {
    schema
        .entities
        .iter()
        .find(|entity| entity.name == name)
        .unwrap_or_else(|| panic!("{name} declared"))
}

fn attribute<'a>(entity: &'a EntityDef, name: &str) -> &'a Attribute {
    entity
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .unwrap_or_else(|| panic!("{}.{name} declared", entity.name))
}

#[test]
fn aggregation_levels_keep_kind_bounds_and_uniqueness_outermost_first() {
    let schema = parse(SHAPES);
    let owner = entity(&schema, "Owner");
    assert_eq!(
        owner
            .attributes
            .iter()
            .map(|attribute| attribute.name.as_str())
            .collect::<Vec<_>>(),
        ["GlobalId", "Code", "Grid", "Maps", "Tags", "Slots", "Knots", "Pairs", "Degree"],
        "INVERSE and UNIQUE declarations are not positional attributes"
    );

    assert_eq!(
        attribute(owner, "Grid"),
        &Attribute::new("Grid", "Length")
            .with_aggregation(
                Aggregation::new(AggregateKind::List, Bound::Integer(2), Bound::Integer(3))
                    .unique()
            )
            .with_aggregation(Aggregation::new(
                AggregateKind::List,
                Bound::Integer(1),
                Bound::Unbounded
            ))
    );
    assert_eq!(
        attribute(owner, "Maps"),
        &Attribute::new("Maps", "Map").with_aggregation(
            Aggregation::new(AggregateKind::List, Bound::Integer(1), Bound::Unbounded).unique()
        ),
        "keywords are case-insensitive"
    );
    assert_eq!(
        attribute(owner, "Tags"),
        &Attribute::new("Tags", "Label")
            .optional()
            .with_aggregation(Aggregation::new(
                AggregateKind::Set,
                Bound::Integer(0),
                Bound::Unbounded
            )),
        "omitted bounds are [0:?]"
    );
    assert_eq!(
        attribute(owner, "Slots"),
        &Attribute::new("Slots", "Length").with_aggregation(
            Aggregation::new(AggregateKind::Array, Bound::Integer(1), Bound::Integer(3))
                .optional_elements()
        ),
        "OPTIONAL elements do not make the attribute optional"
    );
    assert_eq!(
        attribute(owner, "Knots").aggregation[0].upper,
        Bound::Expression("SELF\\Owner.Degree".into())
    );
    assert_eq!(
        attribute(owner, "Pairs").aggregation[0].kind,
        AggregateKind::Bag
    );
    assert!(attribute(owner, "Degree").aggregation.is_empty());
    assert!(!attribute(owner, "Degree").aggregate);
}

#[test]
fn inverse_attributes_record_target_for_attribute_and_bounds() {
    let schema = parse(SHAPES);
    let owner = entity(&schema, "Owner");
    assert_eq!(
        owner.inverses,
        [
            InverseAttribute::new("Parts", "Part", "Whole").with_aggregation(Aggregation::new(
                AggregateKind::Set,
                Bound::Integer(0),
                Bound::Unbounded
            )),
            InverseAttribute::new("Host", "Part", "Guest"),
            InverseAttribute::new("Uses", "Part", "Part.Used").with_aggregation(Aggregation::new(
                AggregateKind::Bag,
                Bound::Integer(1),
                Bound::Integer(5)
            )),
        ]
    );
    assert_eq!(owner.derived, ["Count"]);
    assert_eq!(owner.where_rules.len(), 1);

    let special = entity(&schema, "Special");
    assert_eq!(special.inverses.len(), 1);
    assert_eq!(special.inverses[0].name, "Parts");
    assert_eq!(special.inverses[0].redeclares.as_deref(), Some("Owner"));
    assert!(special.attributes.is_empty());
}

#[test]
fn unique_rules_keep_labels_and_names_as_written() {
    let schema = parse(SHAPES);
    let owner = entity(&schema, "Owner");
    assert_eq!(
        owner.unique_rules[..2],
        [
            UniqueRule::new("UR1", vec!["GlobalId".into()]),
            UniqueRule::new("UR2", vec!["GlobalId".into(), "SELF\\Owner.Code".into()]),
        ]
    );
    assert_eq!(owner.unique_rules[2].label, None);
    assert_eq!(owner.unique_rules[2].attributes, ["Code"]);
}

/// The three IFC schemas, when a local copy is configured
/// (`STEP_IFC_SCHEMA_DIR` containing `ifc2x3-tc1/IFC2X3_TC1.exp`,
/// `ifc4-add2-tc1/IFC4.exp` and `ifc4x3-add2/IFC4X3_ADD2.exp`, the layout
/// `openbimrs/ifc`'s `scripts/fetch-ifc-schemas.sh` produces).
///
/// The declaration counts were checked against an independent count of the
/// schema text (statements between a line-level `INVERSE` or `UNIQUE` and the
/// next block keyword).
#[test]
fn ifc_schemas_expose_inverses_unique_rules_and_bounds() {
    let Some(dir) = std::env::var_os("STEP_IFC_SCHEMA_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    for (path, inverses, unique_rules, decomposed_by) in [
        ("ifc2x3-tc1/IFC2X3_TC1.exp", 115, 17, "IfcRelDecomposes"),
        ("ifc4-add2-tc1/IFC4.exp", 153, 4, "IfcRelAggregates"),
        ("ifc4x3-add2/IFC4X3_ADD2.exp", 165, 4, "IfcRelAggregates"),
    ] {
        let schema = parse(&std::fs::read_to_string(dir.join(path)).expect("schema readable"));
        let count = |f: fn(&EntityDef) -> usize| schema.entities.iter().map(f).sum::<usize>();
        assert_eq!(count(|e| e.inverses.len()), inverses, "{path}");
        assert_eq!(count(|e| e.unique_rules.len()), unique_rules, "{path}");

        assert_eq!(
            attribute(entity(&schema, "IfcCartesianPoint"), "Coordinates").aggregation,
            [Aggregation::new(
                AggregateKind::List,
                Bound::Integer(1),
                Bound::Integer(3)
            )],
            "{path}"
        );
        assert_eq!(
            entity(&schema, "IfcRoot").unique_rules,
            [UniqueRule::new("UR1", vec!["GlobalId".into()])],
            "{path}"
        );
        assert_eq!(
            entity(&schema, "IfcApplication").unique_rules.last(),
            Some(&UniqueRule::new(
                "UR2",
                vec!["ApplicationFullName".into(), "Version".into()]
            )),
            "{path}"
        );
        let decomposed = entity(&schema, "IfcObjectDefinition")
            .inverses
            .iter()
            .find(|inverse| inverse.name == "IsDecomposedBy")
            .expect("IsDecomposedBy");
        assert_eq!(decomposed.entity, decomposed_by, "{path}");
        assert_eq!(decomposed.for_attribute, "RelatingObject", "{path}");
        assert_eq!(
            decomposed.aggregation,
            Some(Aggregation::new(
                AggregateKind::Set,
                Bound::Integer(0),
                Bound::Unbounded
            )),
            "{path}"
        );
        // The UNIQUE inside RepresentationMaps' type must not open a block.
        let type_product = entity(&schema, "IfcTypeProduct");
        assert!(attribute(type_product, "RepresentationMaps").aggregation[0].unique);
        assert!(type_product.attributes.iter().any(|a| a.name == "Tag"));
    }

    let ifc4 = parse(&std::fs::read_to_string(dir.join("ifc4-add2-tc1/IFC4.exp")).unwrap());
    assert_eq!(
        attribute(entity(&ifc4, "IfcCartesianPointList3D"), "CoordList"),
        &Attribute::new("CoordList", "IfcLengthMeasure")
            .with_aggregation(Aggregation::new(
                AggregateKind::List,
                Bound::Integer(1),
                Bound::Unbounded
            ))
            .with_aggregation(Aggregation::new(
                AggregateKind::List,
                Bound::Integer(3),
                Bound::Integer(3)
            )),
        "nested lists no longer collapse to one"
    );
}
