//! Malformed-record recovery: strict by default, opt-in skip with diagnostics.

use openbim_step::{parse, parse_with, Diagnostic, Exchange, OnMalformed, ParseOptions, Severity};

/// A minimal well-formed exchange with `records` spliced into DATA.
fn exchange(records: &str) -> String {
    format!(
        "ISO-10303-21;\n\
         HEADER;\n\
         FILE_DESCRIPTION((''),'2;1');\n\
         FILE_NAME('n','t',(''),(''),'p','o','a');\n\
         FILE_SCHEMA(('IFC4'));\n\
         ENDSEC;\n\
         DATA;\n\
         {records}\n\
         ENDSEC;\n\
         END-ISO-10303-21;\n"
    )
}

/// The exact defect in AC20-FZK-Haus.ifc: a truncated write that ate the
/// middle of one record, leaving `#7` immediately followed by the tail of the
/// next entity.
const TRUNCATED_WRITE: &str = "#79106= IFCCONNECTIONSURFACEGEOMETRY(#79104,$);\n\
     #7\n\
     ACEBOUNDARY('13UjdmCIGNmNY28Gtm7OlY',#12,'2ndLevel','2a',#76214,#67536,#79106,.PHYSICAL.,.EXTERNAL.);";

fn ids(exchange: &Exchange) -> Vec<String> {
    exchange
        .data
        .records
        .iter()
        .map(|record| record.id.as_str().to_string())
        .collect()
}

#[test]
fn malformed_records_abort_by_default() {
    let input = exchange(TRUNCATED_WRITE);
    let error = parse(input.as_bytes()).expect_err("strict parsing must reject a damaged record");
    assert!(error.detail().contains('='), "{}", error.detail());

    // The explicit strict policy must behave identically to the default.
    assert_eq!(
        parse_with(input.as_bytes(), ParseOptions::strict()).map(|outcome| outcome.exchange),
        Err(error)
    );
}

#[test]
fn skip_policy_recovers_the_surrounding_records_and_reports_the_gap() {
    let input = exchange(TRUNCATED_WRITE);
    let outcome = parse_with(input.as_bytes(), ParseOptions::lenient())
        .expect("recovery must not fail the whole file");

    assert_eq!(ids(&outcome.exchange), ["79106"]);
    assert!(!outcome.is_lossless());
    assert_eq!(outcome.diagnostics.len(), 1);

    let diagnostic: &Diagnostic = &outcome.diagnostics[0];
    assert_eq!(diagnostic.severity(), Severity::Warning);
    assert!(
        diagnostic
            .detail()
            .contains("skipped malformed data record"),
        "{}",
        diagnostic.detail()
    );

    // The span must quote the exact discarded bytes so a consumer can show
    // them, and must not extend into the following structure.
    let dropped = &input.as_bytes()[diagnostic.span().start..diagnostic.span().end];
    let dropped = String::from_utf8_lossy(dropped);
    assert!(dropped.starts_with("#7"), "{dropped}");
    assert!(dropped.trim_end().ends_with(';'), "{dropped}");
    assert!(!dropped.contains("ENDSEC"), "{dropped}");
}

#[test]
fn recovery_keeps_every_record_after_the_damaged_one() {
    let input = exchange(&format!(
        "#1= IFCPERSON($,$,'a',$,$,$,$,$);\n\
         {TRUNCATED_WRITE}\n\
         #90= IFCORGANIZATION($,'o',$,$,$);\n\
         #91= IFCPERSONANDORGANIZATION(#1,#90,$);"
    ));
    let outcome = parse_with(input.as_bytes(), ParseOptions::lenient()).expect("recovery");

    assert_eq!(ids(&outcome.exchange), ["1", "79106", "90", "91"]);
    assert_eq!(outcome.diagnostics.len(), 1);
}

#[test]
fn a_clean_file_reports_no_diagnostics_under_either_policy() {
    let input = exchange("#1= IFCPERSON($,$,'a',$,$,$,$,$);\n#2= IFCORGANIZATION($,'o',$,$,$);");
    for options in [ParseOptions::strict(), ParseOptions::lenient()] {
        let outcome = parse_with(input.as_bytes(), options).expect("clean file");
        assert_eq!(ids(&outcome.exchange), ["1", "2"]);
        assert!(outcome.is_lossless());
    }
    assert_eq!(
        parse(input.as_bytes()).expect("clean file"),
        parse_with(input.as_bytes(), ParseOptions::lenient())
            .expect("clean file")
            .exchange
    );
}

#[test]
fn recovery_does_not_consume_a_semicolon_inside_a_string_or_comment() {
    let input = exchange(
        "#1= IFCPERSON($,$,'a;b',$,$,$,$,$); /*;;; ENDSEC; ;;*/\n\
         #2 IFCORGANIZATION($,'o; ENDSEC;',$,$,$);\n\
         #3= IFCPERSON($,$,'c',$,$,$,$,$);",
    );
    let outcome = parse_with(input.as_bytes(), ParseOptions::lenient()).expect("recovery");

    assert_eq!(ids(&outcome.exchange), ["1", "3"]);
    assert_eq!(outcome.diagnostics.len(), 1);
    let dropped = String::from_utf8_lossy(
        &input.as_bytes()[outcome.diagnostics[0].span().start..outcome.diagnostics[0].span().end],
    );
    assert!(dropped.contains("ENDSEC;'"), "{dropped}");
    assert!(dropped.trim_end().ends_with(");"), "{dropped}");
}

#[test]
fn recovery_stops_at_the_section_end_instead_of_swallowing_it() {
    // The final record is truncated with no terminating `;`, so resynchronizing
    // must stop at ENDSEC rather than consuming the rest of the file.
    let input = exchange("#1= IFCPERSON($,$,'a',$,$,$,$,$);\n#2 IFCORGANIZATION($,'o'");
    let outcome = parse_with(input.as_bytes(), ParseOptions::lenient()).expect("recovery");

    assert_eq!(ids(&outcome.exchange), ["1"]);
    assert_eq!(outcome.diagnostics.len(), 1);
    let dropped = String::from_utf8_lossy(
        &input.as_bytes()[outcome.diagnostics[0].span().start..outcome.diagnostics[0].span().end],
    );
    assert!(!dropped.contains("ENDSEC"), "{dropped}");
}

#[test]
fn header_defects_stay_fatal_under_the_skip_policy() {
    // A consumer opting into record recovery must not silently accept a file
    // whose header identity is unknown.
    let missing_schema = "ISO-10303-21;\n\
         HEADER;\n\
         FILE_DESCRIPTION((''),'2;1');\n\
         FILE_NAME('n','t',(''),(''),'p','o','a');\n\
         ENDSEC;\n\
         DATA;\n\
         #1= IFCPERSON($,$,'a',$,$,$,$,$);\n\
         ENDSEC;\n\
         END-ISO-10303-21;\n";
    let error = parse_with(missing_schema.as_bytes(), ParseOptions::lenient())
        .expect_err("header structure is not recoverable");
    assert!(error.detail().contains("mandatory"), "{}", error.detail());

    let not_step = "#1= IFCPERSON($,$,'a',$,$,$,$,$);\n";
    assert!(parse_with(not_step.as_bytes(), ParseOptions::lenient())
        .expect_err("marker is not recoverable")
        .is_not_step());
}

#[test]
fn lexical_damage_outside_the_data_section_stays_fatal() {
    // Recovery is scoped to DATA payload. A damaged byte in HEADER corrupts
    // the file's identity, so skipping it would yield a model whose
    // provenance is unknown; the same holds between sections.
    let damaged_header = "ISO-10303-21;\n\
         HEADER;\n\
         FILE_DESCRIPTION((''),'2;1');\n\
         FILE_NAME('n','t',(''),(''),'p','o'@,'a');\n\
         FILE_SCHEMA(('IFC4'));\n\
         ENDSEC;\n\
         DATA;\n\
         #1= IFCPERSON($,$,'a',$,$,$,$,$);\n\
         ENDSEC;\n\
         END-ISO-10303-21;\n";
    let error = parse_with(damaged_header.as_bytes(), ParseOptions::lenient())
        .expect_err("header damage is not recoverable");
    assert!(error.detail().contains("0x40"), "{}", error.detail());

    let damaged_between_sections = "ISO-10303-21;\n\
         HEADER;\n\
         FILE_DESCRIPTION((''),'2;1');\n\
         FILE_NAME('n','t',(''),(''),'p','o','a');\n\
         FILE_SCHEMA(('IFC4'));\n\
         ENDSEC;\n\
         DATA;\n\
         #1= IFCPERSON($,$,'a',$,$,$,$,$);\n\
         ENDSEC;\n\
         @;\n\
         END-ISO-10303-21;\n";
    assert!(
        parse_with(damaged_between_sections.as_bytes(), ParseOptions::lenient()).is_err(),
        "damage outside a section must not be recovered"
    );
}

#[test]
fn a_bare_instance_id_before_endsec_does_not_swallow_the_section() {
    // The error span for `#2` covers the ENDSEC token that follows it.
    // Resynchronizing past the span would consume the section terminator and
    // fail the whole file at END-ISO-10303-21.
    let input = exchange("#1= IFCPERSON($,$,'a',$,$,$,$,$);\n#2");
    let outcome = parse_with(input.as_bytes(), ParseOptions::lenient()).expect("recovery");

    assert_eq!(ids(&outcome.exchange), ["1"]);
    assert_eq!(outcome.diagnostics.len(), 1);
    let dropped = String::from_utf8_lossy(
        &input.as_bytes()[outcome.diagnostics[0].span().start..outcome.diagnostics[0].span().end],
    );
    assert!(!dropped.contains("ENDSEC"), "{dropped}");
}

#[test]
fn a_data_section_of_only_damage_yields_an_empty_model_and_diagnostics() {
    let input = exchange("#1 IFCPERSON($,$,'a',$,$,$,$,$);\n#2 IFCORGANIZATION($,'o',$,$,$);");
    let outcome = parse_with(input.as_bytes(), ParseOptions::lenient()).expect("recovery");

    assert!(outcome.exchange.data.records.is_empty());
    assert_eq!(outcome.diagnostics.len(), 2);
}

#[test]
fn options_are_composable_and_strict_by_default() {
    assert_eq!(ParseOptions::default(), ParseOptions::strict());
    assert_eq!(
        ParseOptions::default().on_malformed_record(OnMalformed::Skip),
        ParseOptions::lenient()
    );
    assert_eq!(OnMalformed::default(), OnMalformed::Abort);
}
