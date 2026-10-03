#![allow(missing_docs)]

//! Reals written without the decimal point ISO 10303-21 requires before an
//! exponent (`1E-05`). Strict reading refuses them with a typed error;
//! `ParseOptions::accept_real_without_point` reads them as the real they mean,
//! with the point inserted and one warning each, in the eager, lazy and
//! parallel paths alike.

use openbim_step::{
    decode_record, decode_record_borrowed_with, decode_record_with, parse, parse_parallel_with,
    parse_with, scan, write_to_string, Diagnostic, DiagnosticKind, OnMalformed, Parameter,
    ParseOptions, Span,
};
use std::fmt::Write as _;

fn exchange(records: &str) -> String {
    format!(
        "ISO-10303-21;\n\
         HEADER;\n\
         FILE_DESCRIPTION((''),'2;1');\n\
         FILE_NAME('n','t',(''),(''),'p','o','a');\n\
         FILE_SCHEMA(('IFC4X3'));\n\
         ENDSEC;\n\
         DATA;\n\
         {records}\n\
         ENDSEC;\n\
         END-ISO-10303-21;\n"
    )
}

fn accepting() -> ParseOptions {
    ParseOptions::strict().accept_real_without_point(true)
}

/// Byte span of the first occurrence of `needle` in `haystack`.
fn span_of(haystack: &str, needle: &str) -> Span {
    let start = haystack.find(needle).expect("needle present");
    Span::new(start, start + needle.len())
}

/// The written numbers and the reals they are read as.
const CASES: [(&str, &str); 5] = [
    ("1E-05", "1.E-05"),
    ("-2E3", "-2.E3"),
    ("3e+2", "3.e+2"),
    ("+4E0", "+4.E0"),
    ("10E2", "10.E2"),
];

#[test]
fn strict_parsing_refuses_with_a_typed_error_naming_the_number() {
    for (written, _) in CASES {
        let input = exchange(&format!("#1=IFCREAL({written});"));
        let error = parse(input.as_bytes()).expect_err(written);
        assert!(error.is_real_without_point(), "{written}: {error}");
        assert_eq!(error.span(), span_of(&input, written), "{written}");
        assert!(error.detail().contains(written), "{written}: {error}");
    }
}

#[test]
fn accepted_reals_carry_the_point_and_one_warning_each() {
    for (written, read) in CASES {
        let input = exchange(&format!("#1=IFCREAL({written});"));
        let outcome = parse_with(input.as_bytes(), accepting()).expect(written);
        let record = outcome.exchange.data.records[0]
            .as_simple()
            .expect("simple");
        assert_eq!(record.parameters.as_ref(), [Parameter::Real(read.into())]);

        assert_eq!(outcome.diagnostics.len(), 1, "{written}");
        let diagnostic = &outcome.diagnostics[0];
        assert_eq!(diagnostic.kind(), DiagnosticKind::RealWithoutPoint);
        assert_eq!(diagnostic.span(), span_of(&input, written));
        assert!(diagnostic.detail().contains(written));
        assert!(outcome.is_lossless(), "the record is kept");
    }
}

#[test]
fn the_lenient_preset_accepts_them_instead_of_skipping_the_record() {
    let input = exchange("#1=IFCALIGNMENTSEGMENT(1E-05,(2.,-2E3));");
    let outcome = parse_with(input.as_bytes(), ParseOptions::lenient()).expect("lenient");
    assert_eq!(outcome.exchange.data.records.len(), 1);
    assert_eq!(
        outcome
            .diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.kind(), diagnostic.span()))
            .collect::<Vec<_>>(),
        [
            (DiagnosticKind::RealWithoutPoint, span_of(&input, "1E-05")),
            (DiagnosticKind::RealWithoutPoint, span_of(&input, "-2E3")),
        ],
        "in source order"
    );
    // Skipping alone, without accepting, still loses the record.
    let skipping = ParseOptions::strict().on_malformed_record(OnMalformed::Skip);
    let outcome = parse_with(input.as_bytes(), skipping).expect("skip");
    assert!(outcome.exchange.data.records.is_empty());
    assert_eq!(outcome.diagnostics[0].kind(), DiagnosticKind::SkippedRecord);
}

#[test]
fn incomplete_numbers_stay_errors_when_accepting() {
    for written in ["1E", "1E-", "1EE2", ".5E2", "1E+"] {
        let input = exchange(&format!("#1=IFCREAL({written});"));
        let error = parse_with(input.as_bytes(), accepting()).expect_err(written);
        assert!(!error.is_real_without_point(), "{written}: {error}");
    }
}

#[test]
fn text_and_well_formed_numbers_are_untouched() {
    let input = exchange("#1=IFCX('1E-05',1.E-05,1.5E3,2.,7,-3);");
    let strict = parse(input.as_bytes()).expect("valid Part 21");
    let outcome = parse_with(input.as_bytes(), accepting()).expect("accepting");
    assert_eq!(outcome.exchange, strict);
    assert!(outcome.diagnostics.is_empty());
    let record = strict.data.records[0].as_simple().expect("simple");
    assert_eq!(record.parameters[0], Parameter::Text("1E-05".into()));
}

#[test]
fn ignored_controls_inside_the_number_are_dropped_before_the_point_goes_in() {
    let input = exchange("#1=IFCREAL(1\n2E\r\n-05);");
    let outcome = parse_with(input.as_bytes(), accepting()).expect("accepting");
    let record = outcome.exchange.data.records[0]
        .as_simple()
        .expect("simple");
    assert_eq!(
        record.parameters.as_ref(),
        [Parameter::Real("12.E-05".into())]
    );
    assert_eq!(
        outcome.diagnostics[0].span(),
        span_of(&input, "1\n2E\r\n-05")
    );
}

#[test]
fn the_written_model_is_valid_part21() {
    let input = exchange("#1=IFCREAL(1E-05);");
    let outcome = parse_with(input.as_bytes(), accepting()).expect("accepting");
    let written = write_to_string(&outcome.exchange).expect("writable");
    assert!(written.contains("IFCREAL(1.E-05)"), "{written}");
    assert_eq!(
        parse(written.as_bytes()).expect("strictly valid"),
        outcome.exchange
    );
}

#[test]
fn the_header_stays_strict() {
    let input = exchange("#1=IFCREAL(1.);").replace("'2;1')", "'2;1',1E2)");
    let error = parse_with(input.as_bytes(), ParseOptions::lenient()).expect_err("header");
    assert!(error.is_real_without_point(), "{error}");
}

#[test]
fn a_skipped_record_reports_only_the_skip() {
    let input = exchange("#1=IFCREAL(1E-05,;\n#2=IFCREAL(2E1);");
    let outcome = parse_with(input.as_bytes(), ParseOptions::lenient()).expect("lenient");
    assert_eq!(
        outcome
            .diagnostics
            .iter()
            .map(Diagnostic::kind)
            .collect::<Vec<_>>(),
        [
            DiagnosticKind::SkippedRecord,
            DiagnosticKind::RealWithoutPoint
        ]
    );
    assert_eq!(outcome.diagnostics[1].span(), span_of(&input, "2E1"));
    assert_eq!(outcome.exchange.data.records.len(), 1);
}

#[test]
fn lazy_decoding_matches_the_eager_parse() {
    let input = exchange("#1=IFCREAL(1E-05);\n#2=IFCPAIR(2.,(3e+2,'4E4'));\n#3=IFCREAL(5.);");
    let eager = parse_with(input.as_bytes(), accepting()).expect("eager");
    let scanned = scan(input.as_bytes()).expect("scan frames records without lexing them");
    let records: Vec<_> = scanned
        .records()
        .collect::<Result<_, _>>()
        .expect("framing");
    assert_eq!(records.len(), 3);

    let mut diagnostics = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let (decoded, found) = scanned.decode_with(record, accepting()).expect("decode");
        assert_eq!(decoded, eager.exchange.data.records[index]);
        let (borrowed, found_borrowed) =
            decode_record_borrowed_with(input.as_bytes(), record.span, accepting())
                .expect("borrowed");
        assert_eq!(borrowed.id, decoded.id);
        assert_eq!(found_borrowed, found);
        assert_eq!(
            decode_record_with(input.as_bytes(), record.span, accepting()).expect("free fn"),
            (decoded, found.clone())
        );
        diagnostics.extend(found);
    }
    assert_eq!(diagnostics, eager.diagnostics);

    let strict = decode_record(input.as_bytes(), records[0].span).expect_err("strict");
    assert!(strict.is_real_without_point());
    assert_eq!(strict.span(), span_of(&input, "1E-05"));
}

#[test]
fn the_parallel_parse_equals_the_sequential_one() {
    let mut records = String::new();
    for id in 1..=4000 {
        writeln!(records, "#{id}=IFCREAL({}E-0{});", id % 97, id % 7).expect("String");
    }
    let input = exchange(&records);
    for options in [accepting(), ParseOptions::lenient().check_references(true)] {
        let sequential = parse_with(input.as_bytes(), options).expect("sequential");
        assert_eq!(sequential.diagnostics.len(), 4000);
        let parallel = parse_parallel_with(input.as_bytes(), options, 4).expect("parallel");
        assert_eq!(parallel, sequential);
    }
}
