#![allow(missing_docs)]

use openbim_step::{
    is_step_file, parse, write_parameter, write_to_string, DataRecord, InstanceId, Parameter,
};

const SAMPLE: &[u8] = br#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('Structural test'),'2;1');
FILE_NAME('sample.stp','2026-08-25T10:00:00',('Ada'),('Example Org'),'pre','system','auth');
FILE_SCHEMA(('EXAMPLE_SCHEMA'));
VENDOR_EXTENSION('keep me',WRAPPED((1,.X.)));
ENDSEC;
DATA;
#7= WIDGET($,*,.T.,.F.,.U.,42,-1.5,'it''s \X2\30D3\X0\',"0AF",.BLUE.,#3,(1,2.),LENGTH(0.2));
#3= UNKNOWN_RECORD('preserve');
ENDSEC;
END-ISO-10303-21;
"#;

#[test]
fn detects_magic_with_bom_and_whitespace() {
    assert!(is_step_file(b"\xEF\xBB\xBF\r\n ISO-10303-21;"));
    assert!(is_step_file(b"ISO-10303-\n21;"));
    assert!(!is_step_file(b"ISO-10303-28;"));
}

#[test]
fn physical_file_markers_are_exact_and_ordered() {
    let valid = "ISO-10303-21;HEADER;FILE_DESCRIPTION(('x'),'2;1');FILE_NAME('x','',(),(),'','','');FILE_SCHEMA(('X'));ENDSEC;DATA;#1=X($);ENDSEC;END-ISO-10303-21;";
    parse(valid.as_bytes()).unwrap();

    for invalid in [
        valid.replacen("ISO-10303-21;", "JUNK;ISO-10303-21;", 1),
        valid.replace("END-ISO-10303-21;", "WRONG-END;"),
        format!("{valid}JUNK;"),
    ] {
        assert!(parse(invalid.as_bytes()).is_err(), "accepted {invalid}");
    }
}

#[test]
fn mandatory_header_records_are_present_once_and_in_order() {
    let valid = "ISO-10303-21;HEADER;FILE_DESCRIPTION(('x'),'2;1');FILE_NAME('x','',(),(),'','','');FILE_SCHEMA(('X'));ENDSEC;DATA;#1=X($);ENDSEC;END-ISO-10303-21;";
    let invalid_inputs = [
        valid.replace("FILE_DESCRIPTION(('x'),'2;1');", ""),
        valid.replace(
            "FILE_DESCRIPTION(('x'),'2;1');FILE_NAME('x','',(),(),'','','');",
            "FILE_NAME('x','',(),(),'','','');FILE_DESCRIPTION(('x'),'2;1');",
        ),
        valid.replace(
            "FILE_SCHEMA(('X'));",
            "FILE_SCHEMA(('X'));FILE_SCHEMA(('X'));",
        ),
    ];
    for input in invalid_inputs {
        assert!(parse(input.as_bytes()).is_err(), "accepted {input}");
    }

    let exchange = parse(valid.as_bytes()).unwrap();
    let invalid_headers = [
        exchange.header.records[1..].to_vec(),
        vec![
            exchange.header.records[1].clone(),
            exchange.header.records[0].clone(),
            exchange.header.records[2].clone(),
        ],
        vec![
            exchange.header.records[0].clone(),
            exchange.header.records[1].clone(),
            exchange.header.records[2].clone(),
            exchange.header.records[2].clone(),
        ],
    ];
    for records in invalid_headers {
        let mut invalid = exchange.clone();
        invalid.header.records = records;
        assert!(write_to_string(&invalid).is_err());
    }
}

#[test]
fn semantic_parse_write_reparse_preserves_all_records() {
    let exchange = parse(SAMPLE).expect("sample parses");
    assert_eq!(exchange.header.records.len(), 4);
    assert_eq!(exchange.data.records.len(), 2);
    assert_eq!(exchange.data.records[0].id, InstanceId::from(7_u64));

    let values = &exchange.data.records[0].records[0].parameters;
    assert_eq!(values[0], Parameter::Null);
    assert_eq!(values[1], Parameter::Derived);
    assert_eq!(values[2], Parameter::Bool(true));
    assert_eq!(values[3], Parameter::Bool(false));
    assert_eq!(values[4], Parameter::LogicalUnknown);
    assert_eq!(values[5], Parameter::Integer("42".into()));
    assert_eq!(values[6], Parameter::Real("-1.5".into()));
    assert_eq!(values[7].as_text(), Some("it's ビ"));
    assert_eq!(values[9], Parameter::Enum("BLUE".into()));
    assert_eq!(values[10], Parameter::Ref(InstanceId::from(3_u64)));

    let rendered = write_to_string(&exchange).expect("writer succeeds");
    let reparsed = parse(rendered.as_bytes()).expect("writer output reparses");
    assert_eq!(reparsed, exchange);
    assert!(rendered.contains("VENDOR_EXTENSION"));
    assert!(rendered.contains("UNKNOWN_RECORD"));
}

#[test]
fn exposes_standard_header_records_without_discarding_raw_records() {
    let exchange = parse(SAMPLE).unwrap();
    let header = exchange.header.standard();
    assert_eq!(
        header.description.as_deref(),
        Some(["Structural test".to_string()].as_slice())
    );
    assert_eq!(header.implementation_level.as_deref(), Some("2;1"));
    assert_eq!(header.name.as_deref(), Some("sample.stp"));
    assert_eq!(
        header.author.as_deref(),
        Some(["Ada".to_string()].as_slice())
    );
    assert_eq!(
        header.schema.as_deref(),
        Some(["EXAMPLE_SCHEMA".to_string()].as_slice())
    );
}

#[test]
fn arbitrary_precision_numbers_and_complex_instances_roundtrip_lexically() {
    let input = br"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('x'),'2;1');
FILE_NAME('x','',(),(),'','','');
FILE_SCHEMA(('X'));
ENDSEC;
DATA;
#1=(A(123456789012345678901234567890)B(1.234567890123456789E+999));
ENDSEC;
END-ISO-10303-21;
";
    let exchange = parse(input).unwrap();
    let instance = &exchange.data.records[0];
    assert_eq!(instance.records.len(), 2);
    assert_eq!(
        instance.records[0].parameters[0],
        Parameter::Integer("123456789012345678901234567890".into())
    );
    assert_eq!(
        instance.records[1].parameters[0],
        Parameter::Real("1.234567890123456789E+999".into())
    );
    let rendered = write_to_string(&exchange).unwrap();
    assert_eq!(parse(rendered.as_bytes()).unwrap(), exchange);
}

#[test]
fn unbounded_instance_ids_and_user_defined_keywords_roundtrip() {
    let input = br"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('x'),'2;1');
FILE_NAME('x','',(),(),'','','');
FILE_SCHEMA(('X'));
ENDSEC;
DATA;
#184467440737095516160000=!VENDOR(!WRAPPED(1));
ENDSEC;
END-ISO-10303-21;
";
    let exchange = parse(input).unwrap();
    assert_eq!(
        exchange.data.records[0].id,
        InstanceId::new("184467440737095516160000").unwrap()
    );
    let rendered = write_to_string(&exchange).unwrap();
    assert!(rendered.contains("!VENDOR(!WRAPPED(1))"));
    assert_eq!(parse(rendered.as_bytes()).unwrap(), exchange);
}

#[test]
fn writer_rejects_syntax_bearing_record_names() {
    let mut exchange = parse(SAMPLE).unwrap();
    exchange.data.records[0] = DataRecord::simple(
        InstanceId::from(7_u64),
        "WIDGET);#999=INJECTED(".to_string(),
        vec![Parameter::Null],
    );
    assert!(write_to_string(&exchange).is_err());

    exchange.data.records[0] = DataRecord::simple(
        InstanceId::from(7_u64),
        "_BAD".to_string(),
        vec![Parameter::Null],
    );
    assert!(write_to_string(&exchange).is_err());

    exchange.data.records[0] = DataRecord::simple(
        InstanceId::from(7_u64),
        "!BAD_NAME".to_string(),
        vec![Parameter::Null],
    );
    assert!(write_to_string(&exchange).is_err());
}

#[test]
fn writer_rejects_invalid_unescaped_parameter_lexemes() {
    let invalid: Vec<Parameter> = vec![
        Parameter::Integer("1;#2=X()".into()),
        Parameter::Real("NaN".into()),
        Parameter::Real(".5".into()),
        Parameter::Real("1E3".into()),
        Parameter::Binary("0FG".into()),
        Parameter::Enum("X.Y".into()),
        Parameter::Enum("!VENDOR".into()),
        Parameter::Typed {
            type_name: "X)".into(),
            value: Box::new(Parameter::Null),
        },
    ];
    for parameter in invalid {
        assert!(write_parameter(&parameter, &mut Vec::new()).is_err());
    }
}
