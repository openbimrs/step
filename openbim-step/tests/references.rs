//! Opt-in reference-integrity diagnostics (#4): duplicate instance ids and
//! references to ids that are never defined.

use openbim_step::{
    parse_events_with, parse_with, Diagnostic, DiagnosticKind, Event, InstanceId, ParseOptions,
};
use std::fmt::Write as _;

/// A minimal well-formed exchange with `records` spliced into DATA.
fn exchange(records: &str) -> String {
    format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
         FILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\n\
         ENDSEC;\nDATA;\n{records}\nENDSEC;\nEND-ISO-10303-21;\n"
    )
}

fn checked() -> ParseOptions {
    ParseOptions::strict().check_references(true)
}

fn run(records: &str, options: ParseOptions) -> (String, Vec<Diagnostic>) {
    let input = exchange(records);
    let outcome = parse_with(input.as_bytes(), options).expect("syntactically valid");
    (input, outcome.diagnostics)
}

fn kinds(diagnostics: &[Diagnostic]) -> Vec<(DiagnosticKind, String)> {
    diagnostics
        .iter()
        .map(|d| {
            (
                d.kind(),
                d.instance()
                    .map(|id| id.as_str().to_owned())
                    .unwrap_or_default(),
            )
        })
        .collect()
}

/// The exact input from the issue: silent without the option.
#[test]
fn reference_defects_are_silent_unless_checking_is_requested() {
    let records = "#1=A(#9);\n#2=B(1);\n#2=C(2);";
    assert!(run(records, ParseOptions::strict()).1.is_empty());
    assert!(run(records, ParseOptions::lenient()).1.is_empty());
    let (_, diagnostics) = run(records, checked());
    assert_eq!(
        kinds(&diagnostics),
        [
            (DiagnosticKind::DanglingReference, "9".to_owned()),
            (DiagnosticKind::DuplicateId, "2".to_owned()),
        ]
    );
}

#[test]
fn a_duplicate_id_is_reported_once_on_the_later_record() {
    let (input, diagnostics) = run("#2=B(1);\n#2=C(2);\n#2=D(3);", checked());
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    for diagnostic in &diagnostics {
        assert_eq!(diagnostic.kind(), DiagnosticKind::DuplicateId);
        let span = diagnostic.span();
        let text = &input[span.start..span.end];
        assert!(text.starts_with("#2=") && text.ends_with(';'), "{text:?}");
        assert!(
            !text.contains("B(1)"),
            "the first definition is not the defect"
        );
    }
}

#[test]
fn a_dangling_reference_points_at_the_referencing_record() {
    let (input, diagnostics) = run("#1=A((#9,(#9)),#8);\n#2=B(#1);", checked());
    assert_eq!(
        kinds(&diagnostics),
        [
            (DiagnosticKind::DanglingReference, "9".to_owned()),
            (DiagnosticKind::DanglingReference, "8".to_owned()),
        ],
        "one per distinct missing id, nested lists included"
    );
    let span = diagnostics[0].span();
    assert_eq!(&input[span.start..span.end], "#1=A((#9,(#9)),#8);");
}

/// §11.2: an instance "may be referenced before it is defined".
#[test]
fn forward_references_are_not_reported() {
    let (_, diagnostics) = run("#1=A(#3,#2);\n#2=B(#3);\n#3=C();", checked());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn references_inside_typed_parameters_and_complex_instances_are_checked() {
    let (_, diagnostics) = run("#1=A(WRAP(#7));\n#2=(B(#6) C(#1));", checked());
    assert_eq!(
        kinds(&diagnostics),
        [
            (DiagnosticKind::DanglingReference, "7".to_owned()),
            (DiagnosticKind::DanglingReference, "6".to_owned()),
        ]
    );
}

/// Part 21 instance names are numbers: `#07` names the same instance as `#7`.
#[test]
fn ids_compare_numerically_not_lexically() {
    let (_, dangling) = run("#1=A(#07);\n#7=B();", checked());
    assert!(dangling.is_empty(), "{dangling:?}");
    let (_, duplicate) = run("#7=A();\n#007=B();", checked());
    assert_eq!(
        kinds(&duplicate),
        [(DiagnosticKind::DuplicateId, "007".to_owned())]
    );
}

/// Reference defects lose nothing, so they do not make a parse lossy, and the
/// records are all still there.
#[test]
fn reference_defects_keep_every_record_and_the_parse_lossless() {
    let input = exchange("#1=A(#9);\n#2=B(1);\n#2=C(2);");
    let outcome = parse_with(input.as_bytes(), checked()).unwrap();
    assert_eq!(outcome.exchange.data.records.len(), 3);
    assert_eq!(outcome.diagnostics.len(), 2);
    assert!(outcome.is_lossless());
}

/// A record dropped by lenient recovery never defines its id, so references
/// to it are dangling in the model the caller receives.
#[test]
fn a_reference_to_a_skipped_record_is_dangling() {
    let input = exchange("#1=A(@);\n#2=B(#1);");
    let outcome = parse_with(
        input.as_bytes(),
        ParseOptions::lenient().check_references(true),
    )
    .unwrap();
    assert_eq!(
        kinds(&outcome.diagnostics),
        [
            (DiagnosticKind::SkippedRecord, String::new()),
            (DiagnosticKind::DanglingReference, "1".to_owned()),
        ]
    );
    assert!(!outcome.is_lossless());
}

#[test]
fn diagnostics_are_in_source_order() {
    let (_, diagnostics) = run("#1=A(#9);\n#1=B();\n#3=C(#8);", checked());
    let starts: Vec<usize> = diagnostics.iter().map(|d| d.span().start).collect();
    let mut sorted = starts.clone();
    sorted.sort_unstable();
    assert_eq!(starts, sorted);
    assert_eq!(diagnostics.len(), 3);
}

/// The streaming API reports the same diagnostics as the owned parse.
#[test]
fn the_event_api_reports_the_same_diagnostics() {
    let input = exchange("#1=A(#9);\n#2=B(1);\n#2=C(2);");
    let mut records = 0;
    let diagnostics = parse_events_with(
        input.as_bytes(),
        &mut |event: Event| {
            if matches!(event, Event::DataRecord(_)) {
                records += 1;
            }
        },
        checked(),
    )
    .unwrap();
    assert_eq!(records, 3);
    let owned = parse_with(input.as_bytes(), checked()).unwrap().diagnostics;
    assert_eq!(diagnostics, owned);
}

#[test]
fn skipped_record_diagnostics_have_their_kind_and_no_instance() {
    let input = exchange("#1=A(@);");
    let outcome = parse_with(input.as_bytes(), ParseOptions::lenient()).unwrap();
    assert_eq!(outcome.diagnostics[0].kind(), DiagnosticKind::SkippedRecord);
    assert_eq!(outcome.diagnostics[0].instance(), None::<&InstanceId>);
}

/// One aggregation record forward-referencing many ids must stay linear:
/// deduplicating missing ids by scanning a list would make this quadratic.
#[test]
fn a_record_with_many_forward_references_is_checked_in_linear_time() {
    const N: usize = 50_000;
    let list = (2..N + 2)
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut records = format!("#1=REL(({list}),#{});\n", N + 2);
    for id in 2..N + 2 {
        writeln!(records, "#{id}=E();").expect("writing to a String cannot fail");
    }
    let started = std::time::Instant::now();
    let (_, diagnostics) = run(&records, checked());
    assert_eq!(
        kinds(&diagnostics),
        [(DiagnosticKind::DanglingReference, (N + 2).to_string())],
        "every listed id is defined later; only the extra one is missing"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

/// Missing ids inside one aggregate are reported in the order written.
#[test]
fn dangling_references_in_one_list_keep_source_order() {
    let (_, diagnostics) = run("#1=A((#9,#8,(#7,#6)));", checked());
    assert_eq!(
        kinds(&diagnostics),
        ["9", "8", "7", "6"].map(|id| (DiagnosticKind::DanglingReference, id.to_owned()))
    );
}

/// Ids beyond 64 bits are legal Part 21 and must still compare numerically.
#[test]
fn ids_beyond_sixty_four_bits_still_resolve() {
    let big = "123456789012345678901234567890";
    let (_, resolved) = run(&format!("#1=A(#00{big});\n#{big}=B();"), checked());
    assert!(resolved.is_empty(), "{resolved:?}");
    let (_, dangling) = run(&format!("#1=A(#0{big});"), checked());
    assert_eq!(
        kinds(&dangling),
        [(DiagnosticKind::DanglingReference, big.to_owned())],
        "reported in canonical form"
    );
}

/// Each record holding a dangling reference gets its own diagnostic, even
/// when several records point at the same missing id.
#[test]
fn every_record_referencing_a_missing_id_is_reported() {
    let (input, diagnostics) = run("#1=A(#9);\n#2=B(#9);", checked());
    assert_eq!(
        kinds(&diagnostics),
        [
            (DiagnosticKind::DanglingReference, "9".to_owned()),
            (DiagnosticKind::DanglingReference, "9".to_owned()),
        ]
    );
    let spans: Vec<&str> = diagnostics
        .iter()
        .map(|d| &input[d.span().start..d.span().end])
        .collect();
    assert_eq!(spans, ["#1=A(#9);", "#2=B(#9);"]);
}
