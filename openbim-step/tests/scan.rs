#![allow(missing_docs)]
//! `scan` + `decode_record` against `parse`: on every input, the scan and
//! the decode of every scanned record succeed exactly when `parse` succeeds,
//! and then they return exactly the records `parse` returns.

use openbim_step::{
    decode_record, decode_record_borrowed, parse, scan, DataRecord, Span, StepError,
};
use std::fmt::Write as _;

const HEADER: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('d'),'2;1');\n\
FILE_NAME('n','t',('a'),('o'),'p','s','z');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n";
const FOOTER: &str = "ENDSEC;\nEND-ISO-10303-21;\n";

fn wrap(data: &str) -> Vec<u8> {
    format!("{HEADER}{data}{FOOTER}").into_bytes()
}

/// Scans and decodes every record; the first error ends it.
fn scan_decode(input: &[u8]) -> Result<Vec<DataRecord>, StepError> {
    let scanned = scan(input)?;
    let mut records = Vec::new();
    for record in scanned.records() {
        let record = record?;
        let decoded = scanned.decode(&record)?;
        assert_eq!(decoded.id, record.id, "scanned id differs from decoded id");
        let borrowed = decode_record_borrowed(input, record.span)?;
        assert_eq!(borrowed.id, decoded.id);
        match (&record.name, decoded.records()) {
            (Some(name), [simple]) => assert!(name.eq_ignore_ascii_case(&simple.name)),
            (None, _) => assert!(input[record.span.start..record.span.end].contains(&b'(')),
            (Some(_), _) => panic!("simple record name for a complex instance"),
        }
        records.push(decoded);
    }
    Ok(records)
}

/// The invariant this module guarantees.
fn assert_agrees(input: &[u8], label: &str) {
    let parsed = parse(input);
    let scanned = scan_decode(input);
    match (&parsed, &scanned) {
        (Ok(exchange), Ok(records)) => {
            assert!(
                &exchange.data.records == records,
                "{label}: scan+decode records differ from parse"
            );
        }
        (Err(_), Err(_)) => {}
        (Ok(_), Err(error)) => panic!("{label}: parse succeeds, scan+decode fails: {error}"),
        (Err(error), Ok(_)) => panic!("{label}: parse fails ({error}), scan+decode succeeds"),
    }
}

/// Records with every trap for a record scanner: `;`, `'` and `#1=` inside
/// strings and comments, complex instances, typed and nested parameters.
fn traps(n: usize) -> String {
    let mut out = String::new();
    for id in 1..=n {
        let target = (id * 7919) % n + 1;
        let _ = match id % 8 {
            0 => writeln!(out, "#{id}=IFCLABEL('decoy;#{target}=X();');"),
            1 => writeln!(out, "#{id}=IFCWALL(#{target},'n',$,*,(1,2.5,-3.E2),.T.);"),
            2 => writeln!(
                out,
                "/* ;#{target}=COMMENT(); ' */#{id}=IFCSLAB(IFCLENGTHMEASURE(2.),\"0F\");"
            ),
            3 => writeln!(out, "#{id}=(A(1)B('x;#9=Y();')C(#{target}));"),
            4 => writeln!(
                out,
                "#{id}=IFCTEXT('it''s;','Stra\\S\\'e;',/* c; ' */'\\X2\\00E4\\X0\\');"
            ),
            5 => writeln!(out, "#{id} = IFCSPACED ( 1 , 2 ) ;"),
            6 => writeln!(out, "#{id}=ifclower(.enum.,'a\\\\');"),
            _ => writeln!(out, "#{id}=IFCPOINT((0.,{id}.,1.5E-3));"),
        };
    }
    out
}

#[test]
fn clean_files_scan_and_decode_to_the_parsed_records() {
    for n in [0, 1, 8, 100, 2_000] {
        let input = wrap(&traps(n));
        assert!(parse(&input).is_ok(), "fixture n={n} must parse");
        assert_agrees(&input, &format!("traps n={n}"));
        let count = scan(&input).expect("header").records().count();
        assert_eq!(count, n);
    }
}

#[test]
fn scanned_names_and_spans_are_as_written() {
    let input = wrap("#1=IFCWALL('a;b');\n#20 = ifclower();\n#3=(B(2)A(1));\n");
    let scanned = scan(&input).expect("header");
    let records: Vec<_> = scanned.records().collect::<Result<_, _>>().expect("scan");
    let text = |span: Span| std::str::from_utf8(&input[span.start..span.end]).unwrap();
    assert_eq!(records[0].name.as_deref(), Some("IFCWALL"));
    assert_eq!(text(records[0].span), "#1=IFCWALL('a;b');");
    assert_eq!(records[1].id.as_str(), "20");
    assert_eq!(records[1].name.as_deref(), Some("ifclower"));
    assert_eq!(text(records[1].span), "#20 = ifclower();");
    assert_eq!(records[2].name, None);
    assert_eq!(text(records[2].span), "#3=(B(2)A(1));");
}

#[test]
fn lexer_edge_cases_frame_like_the_parser() {
    let cases = [
        // `\S\'`: the quote is payload, not the end of the string.
        "#1=IFCLABEL('Stra\\S\\'e;');\n#2=IFCLABEL('x');\n",
        // Doubled quotes right before `)` and `,`.
        "#1=IFCLABEL('it''s'),#2=X('''');\n",
        "#1=IFCLABEL('a'''),'b';\n",
        // Ignored controls inside an id, a name, a quote pair and a comment
        // opener: legal Part 21, and read by the lexer fallback.
        "#1\r\n2=IFC\nWALL('a'\n'b');\n#13=X(/\n* ; ' *\n/1);\n",
        // A print directive between the two quotes of an escaped quote.
        "#1=IFCLABEL('a'\\N\\'b;');\n",
        // Comments and directives between records and before the id.
        "/* #9=Z(); */ \\N\\ #1=A(); /* ; */\n#2=B();\n",
        // A lone `/` in a record is a syntax error for both.
        "#1=A(1/2);\n#2=B();\n",
        // Binary literals, valid and not.
        "#1=A(\"0F\");\n",
        "#1=A(\"0;\");\n#2=B();\n",
    ];
    for (index, data) in cases.iter().enumerate() {
        assert_agrees(&wrap(data), &format!("case {index}"));
    }
}

#[test]
fn defects_between_and_after_records_are_errors() {
    let broken = [
        // Junk between records: parse rejects it, so must the scan.
        wrap("#1=A();\ngarbage\n#2=B();\n"),
        wrap("#1=A();\n----\n#2=B();\n"),
        // A record without `=` or without a body.
        wrap("#1 A();\n"),
        wrap("#1=;\n"),
        // Unterminated record, string and comment.
        format!("{HEADER}#1=A(1,2").into_bytes(),
        format!("{HEADER}#1=A('open);\n{FOOTER}").into_bytes(),
        format!("{HEADER}#1=A(/* open);\n{FOOTER}").into_bytes(),
        // Missing ENDSEC, missing end marker, content after it.
        format!("{HEADER}#1=A();\nEND-ISO-10303-21;\n").into_bytes(),
        format!("{HEADER}#1=A();\nENDSEC;\n").into_bytes(),
        format!("{HEADER}#1=A();\n{FOOTER}{HEADER}").into_bytes(),
        // Two physical files joined together.
        [wrap("#1=A();\n"), wrap("#1=A();\n")].concat(),
    ];
    for (index, input) in broken.iter().enumerate() {
        assert!(parse(input).is_err(), "fixture {index} must be invalid");
        assert!(
            scan_decode(input).is_err(),
            "broken {index} scanned cleanly"
        );
    }
}

#[test]
fn header_defects_fail_the_scan_itself() {
    assert!(scan(b"not a step file").is_err());
    assert!(scan(b"ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n").is_err());
    assert!(scan(b"ISO-10303-21;\nHEADER;\n").is_err());
}

#[test]
fn decode_rejects_spans_that_are_not_one_record() {
    let input = wrap("#1=A();\n#2=B(1);\n");
    let scanned = scan(&input).expect("header");
    let records: Vec<_> = scanned.records().collect::<Result<_, _>>().expect("scan");
    let (first, second) = (records[0].span, records[1].span);
    assert!(decode_record(&input, first).is_ok());
    // Too short, too long, two records, not at `#`, out of range.
    assert!(decode_record(&input, Span::new(first.start, first.end - 1)).is_err());
    assert!(decode_record(&input, Span::new(first.start, first.end + 1)).is_err());
    assert!(decode_record(&input, Span::new(first.start, second.end)).is_err());
    assert!(decode_record(&input, Span::new(first.start + 1, first.end)).is_err());
    assert!(decode_record(&input, Span::new(first.start, input.len() + 1)).is_err());
    assert!(decode_record(&input, Span::new(first.end, first.start)).is_err());
}

/// Deterministic xorshift, so a failure names a reproducible seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % bound as u64).expect("bound fits usize")
    }
}

#[test]
fn mutated_files_keep_the_invariant() {
    // Bytes that change how a scanner or lexer reads what follows.
    const INSERTS: [&[u8]; 14] = [
        b"\n", b"\r\n", b"\t", b"\x0c", b"'", b"''", b";", b"/*", b"*/", b"\\S\\", b"\\N\\", b"\"",
        b"#", b"/",
    ];
    let base = wrap(&traps(24));
    let data_start = HEADER.len();
    let mut valid = 0;
    for seed in 1..=3_000u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut input = base.clone();
        for _ in 0..=rng.below(3) {
            let at = data_start + rng.below(input.len() - data_start);
            let insert = INSERTS[rng.below(INSERTS.len())];
            input.splice(at..at, insert.iter().copied());
        }
        assert_agrees(&input, &format!("seed {seed}"));
        valid += usize::from(parse(&input).is_ok());
    }
    // Both outcomes must be common, or half of the invariant goes untested.
    assert!(
        (300..=2_700).contains(&valid),
        "{valid} of 3000 mutants parse"
    );
}

#[test]
fn quote_pairs_split_by_controls_or_directives_stay_one_string() {
    // A doubled quote may be split by ignored controls or print directives
    // (0.7.0 behaviour); the scanner's string skip must agree. Pinned values,
    // because a lexer change would move `parse` and `scan` together.
    let cases: [(&str, &[&str]); 4] = [
        ("#1=X('a'\\N\\'b;');", &["a'b;"]),
        ("#1=X('a'\n'b');", &["a'b"]),
        ("#1=X('a'\\F\\\r\n'b','c');", &["a'b", "c"]),
        ("#1=X('a''','b');", &["a'", "b"]),
    ];
    check_pinned(&cases);
}

#[test]
fn backslash_sequences_decode_as_before() {
    // Values from the published 0.7.0: the string skip's backslash fast
    // path must not move where a literal ends or what it decodes to.
    let cases: [(&str, &[&str]); 10] = [
        (r"#1=X('\X2\00E400F6\X0\','\X\E9t');", &["äö", "ét"]),
        (r"#1=X('\F\\S\'');", &["§"]),
        (r"#1=X('a\\S\','b');", &["a\\S\\", "b"]),
        (r"#1=X('a\\b','\\','x\\\S\'');", &["a\\b", "\\", "x\\§"]),
        (r"#1=X('\N\\S\'');", &["§"]),
        (r"#1=X('a\\N\b','c');", &["a\\N\\b", "c"]),
        (
            r"#1=X('\PA\\S\D\X0\\X2\00E4\X0\');",
            &["Ä\\X0\\X2\\00E4\\X0\\"],
        ),
        (r"#1=X('\X4\0001F600\X0\\');", &["😀\\"]),
        ("#1=X('a\\\nS\\'b');", &["a§b"]),
        (r"#1=X('\F\\N\x','\');", &["x", "\\"]),
    ];
    check_pinned(&cases);
}

fn check_pinned(cases: &[(&str, &[&str])]) {
    use openbim_step::Parameter::Text;
    for (data, expected) in cases {
        let input = wrap(&format!("{data}\n"));
        let exchange = parse(&input).expect(data);
        let expected: Box<[_]> = expected.iter().map(|text| Text((*text).into())).collect();
        assert_eq!(
            exchange.data.records[0].records()[0].parameters,
            expected,
            "{data:?}"
        );
        assert_agrees(&input, data);
    }
}
