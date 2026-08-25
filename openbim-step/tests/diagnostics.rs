#![allow(missing_docs)]

use openbim_step::{parse, write_parameter, Parameter, Source, Span};

#[test]
fn syntax_errors_carry_source_spans_and_resolve_to_line_columns() {
    let input = b"ISO-10303-21;\nHEADER;\nBROKEN 'x');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;";
    let error = parse(input).unwrap_err();
    let span = error.span();
    let source = Source::new("broken.stp", input);
    let location = source.location(span);
    assert_eq!(location.source_name, "broken.stp");
    assert_eq!(location.line, 3);
    assert_eq!(location.column, 1);
    assert!(error.detail().contains("FILE_DESCRIPTION"));
    assert!(!location.line_text.is_empty());
}

#[test]
fn not_step_points_at_the_source_start() {
    let error = parse(b"not a physical file").unwrap_err();
    assert!(error.is_not_step());
    assert_eq!(error.span(), Span::new(0, 0));
    assert!(error.to_string().contains("not a STEP physical file"));
}

#[test]
fn parser_and_writer_reject_excessive_parameter_nesting() {
    let nesting = 129;
    let nested = format!("{}${}", "(".repeat(nesting), ")".repeat(nesting));
    let input = format!(
        "ISO-10303-21;HEADER;FILE_DESCRIPTION(('x'),'2;1');FILE_NAME('x','',(),(),'','','');FILE_SCHEMA(('X'));ENDSEC;DATA;#1=X({nested});ENDSEC;END-ISO-10303-21;"
    );
    let error = parse(input.as_bytes()).expect_err("deep nesting must be bounded");
    assert!(error.detail().contains("nesting"));

    let mut parameter: Parameter = Parameter::Null;
    for _ in 0..nesting {
        parameter = Parameter::List(vec![parameter]);
    }
    let error =
        write_parameter(&parameter, &mut Vec::new()).expect_err("writer nesting must be bounded");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}
