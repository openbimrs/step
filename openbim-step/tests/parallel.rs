#![allow(missing_docs)]
//! `parse_parallel_with` returns exactly what `parse_with` returns -- the
//! same exchange, diagnostics in the same order, the same error -- for any
//! thread count, including inputs built so that split guesses land inside
//! strings, comments and damaged records.

use openbim_step::{parse_parallel_with, parse_with, ParseOptions};
use std::fmt::Write as _;

const HEADER: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('d'),'2;1');\n\
FILE_NAME('n','t',('a'),('o'),'p','s','z');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n";
const FOOTER: &str = "ENDSEC;\nEND-ISO-10303-21;\n";

/// A data section of `n` records with every trap for a boundary guess:
/// strings and comments containing `;#1=`, forward references across the
/// whole file, and (when `damage`) malformed records, duplicate ids and
/// dangling references scattered through it.
fn file(n: usize, damage: bool) -> Vec<u8> {
    let mut out = String::from(HEADER);
    for id in 1..=n {
        let target = (id * 7919) % n + 1;
        let record = match id % 9 {
            0 => format!("#{id}=IFCLABEL('decoy;#{target}=X();');\n"),
            1 => format!("#{id}=IFCWALL(#{target},'n',$,*,(1,2.5,-3.E2),.T.);\n"),
            2 => {
                format!("/* ;#{target}=COMMENT(); */#{id}=IFCSLAB(IFCLENGTHMEASURE(2.),\"0F\");\n")
            }
            3 => format!("#{id}=(A(1)B('x;#9=Y();')C(#{target}));\n"),
            4 if damage => format!("#{id}=IFCBROKEN(1,,2);\n"),
            5 if damage => format!("#{target}=IFCDUPLICATE();\n"),
            6 if damage => format!("#{id}=IFCDANGLING(#{});\n", n * 10 + id),
            _ => format!("#{id}=IFCPOINT((0.,{id}.,1.5E-3));\n"),
        };
        out.push_str(&record);
    }
    out.push_str(FOOTER);
    out.into_bytes()
}

fn options() -> [ParseOptions; 2] {
    [
        ParseOptions::strict(),
        ParseOptions::lenient().check_references(true),
    ]
}

fn assert_same(input: &[u8], label: &str) {
    for options in options() {
        let sequential = format!("{:?}", parse_with(input, options));
        for threads in [1, 2, 3, 5, 8, 16, 64] {
            let parallel = format!("{:?}", parse_parallel_with(input, options, threads));
            assert!(
                parallel == sequential,
                "{label}: threads={threads} options={options:?} differs"
            );
        }
    }
}

#[test]
fn clean_files_parse_identically_on_every_thread_count() {
    for n in [1, 2, 9, 100, 2_000] {
        assert_same(&file(n, false), &format!("clean n={n}"));
    }
}

#[test]
fn damaged_files_recover_and_diagnose_identically() {
    for n in [9, 100, 2_000] {
        let input = file(n, true);
        // The damage must actually produce diagnostics under recovery, or
        // this test checks nothing.
        let lenient = parse_with(&input, ParseOptions::lenient().check_references(true))
            .expect("recovering parse succeeds");
        assert!(
            !lenient.diagnostics.is_empty(),
            "n={n} produced no diagnostics"
        );
        assert_same(&input, &format!("damaged n={n}"));
    }
}

/// Recovery can resume before a record head as well as past a `;`. The slices
/// must still land where the sequential parse does.
#[test]
fn records_missing_their_semicolon_recover_identically() {
    for separator in ["\n", " ", ""] {
        let mut records = String::new();
        for id in 1..=2_000 {
            let end = if id % 7 == 0 { "" } else { ";" };
            write!(records, "#{id}=IFCPOINT(({id}.,1.)){end}{separator}")
                .expect("writing to a String cannot fail");
        }
        let input = format!("{HEADER}{records}\n{FOOTER}");
        let outcome = parse_with(input.as_bytes(), ParseOptions::lenient()).unwrap();
        assert_eq!(outcome.diagnostics.len(), 2_000 / 7);
        assert_eq!(outcome.exchange.data.records.len(), 2_000 - 2_000 / 7);
        assert_same(input.as_bytes(), &format!("separator {separator:?}"));
    }
}

/// Every `;` in turn removed from a file full of split-guess traps, including
/// those inside literals, where the removal changes nothing but the payload.
#[test]
fn removing_any_one_semicolon_recovers_identically() {
    let input = file(40, true);
    let data = HEADER.len();
    for position in data..input.len() - FOOTER.len() {
        if input[position] == b';' {
            let mut damaged = input.clone();
            damaged.remove(position);
            assert_same(&damaged, &format!("without the ';' at {position}"));
        }
    }
}

#[test]
fn structural_errors_are_the_sequential_errors() {
    let good = file(500, false);
    let text = String::from_utf8(good).expect("ascii");
    let cases = [
        text.replace("ENDSEC;\nEND-ISO", "END-ISO"),
        text.replacen("#250=", "#250", 1),
        text.replace("END-ISO-10303-21;\n", ""),
        text.replacen("DATA;", "DATA", 1),
        format!("{text}#999=TRAILING();"),
        text[..text.len() / 2].to_owned(),
    ];
    for (index, case) in cases.iter().enumerate() {
        assert_same(case.as_bytes(), &format!("error case {index}"));
    }
}

#[test]
fn not_step_and_empty_inputs_match() {
    assert_same(b"", "empty");
    assert_same(b"not a step file", "not step");
    assert_same(format!("{HEADER}{FOOTER}").as_bytes(), "no records");
}

/// Recovery diagnostics must appear where the sequential parse puts them:
/// a skipped record's diagnostic before the diagnostics of later records.
/// Built so that skipped records and duplicate ids alternate and so every
/// slice sees both, which fixes one exact interleaving.
#[test]
fn skipped_and_duplicate_diagnostics_interleave_in_source_order() {
    let mut out = String::from(HEADER);
    for id in 1..=400 {
        writeln!(out, "#{id}=IFCPOINT(({id}.,0.));").expect("writing to a String cannot fail");
        if id % 10 == 0 {
            writeln!(out, "#{}=IFCBROKEN(,);", 10_000 + id)
                .expect("writing to a String cannot fail");
            writeln!(out, "#{id}=IFCDUPLICATE();").expect("writing to a String cannot fail");
        }
    }
    out.push_str(FOOTER);
    let options = ParseOptions::lenient().check_references(true);
    let sequential = parse_with(out.as_bytes(), options).expect("recovers");
    let kinds: Vec<_> = sequential
        .diagnostics
        .iter()
        .take(4)
        .map(openbim_step::Diagnostic::kind)
        .collect();
    // Guard: the fixture really interleaves skip and duplicate diagnostics.
    assert_ne!(kinds[0], kinds[1], "fixture does not interleave: {kinds:?}");
    // A skipped record reports its id, which the parallel parse must keep.
    assert_eq!(
        sequential.diagnostics[0]
            .instance()
            .map(openbim_step::InstanceId::as_str),
        Some("10010")
    );
    assert_same(out.as_bytes(), "interleaved diagnostics");
}

/// The header comes from the prefix parse (which must stop at `DATA;`) and
/// the records from the slices: each record exactly once, header unchanged.
#[test]
fn the_prefix_parse_stops_at_data_and_its_output_is_kept() {
    let input = file(3_000, false);
    let options = ParseOptions::strict();
    let parallel = parse_parallel_with(&input, options, 8).expect("parses");
    let sequential = parse_with(&input, options).expect("parses");
    assert_eq!(parallel.exchange.header, sequential.exchange.header);
    assert_eq!(parallel.exchange.data.records.len(), 3_000);
    assert_eq!(parallel, sequential);
}
