#![allow(missing_docs)]
//! The 0.9 model shape: simple instances inline, complex instances as
//! written, shared names in the owned model, boxed parameter lists.

use openbim_step::{
    decode_record, parse, parse_parallel_with, scan, write_to_string, DataRecord, Instance,
    InstanceId, Parameter, ParseOptions, Record, Str,
};

const HEADER: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('d'),'2;1');\n\
FILE_NAME('n','t',('a'),('o'),'p','s','z');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n";
const FOOTER: &str = "ENDSEC;\nEND-ISO-10303-21;\n";

fn wrap(data: &str) -> Vec<u8> {
    format!("{HEADER}{data}{FOOTER}").into_bytes()
}

#[test]
fn simple_and_complex_instances_keep_their_written_form() {
    let input = wrap("#1=A(1);\n#2=(A(1));\n#3=(A(1)B(2));\n");
    let exchange = parse(&input).expect("parses");
    let [one, two, three] = &exchange.data.records[..] else {
        panic!("three records");
    };
    assert!(matches!(one.instance, Instance::Simple(_)));
    assert!(matches!(&two.instance, Instance::Complex(records) if records.len() == 1));
    assert!(matches!(&three.instance, Instance::Complex(records) if records.len() == 2));
    // `records()` reads both shapes the same way.
    assert_eq!(one.records(), two.records());
    assert_eq!(one.as_simple(), Some(&one.records()[0]));
    assert_eq!(two.as_simple(), None);

    let written = write_to_string(&exchange).expect("writes");
    assert!(written.contains("#1=A(1);"), "{written}");
    assert!(written.contains("#2=(A(1));"), "{written}");
    assert!(written.contains("#3=(A(1)B(2));"), "{written}");
    assert_eq!(parse(written.as_bytes()).expect("reparses"), exchange);
}

#[test]
fn the_writer_refuses_an_empty_complex_instance() {
    let mut exchange = parse(&wrap("#1=A(1);\n")).expect("parses");
    exchange.data.records[0] = DataRecord::complex(InstanceId::from(1_u64), Vec::new());
    assert!(write_to_string(&exchange).is_err());
}

#[test]
fn short_values_are_inline_and_long_names_shared() {
    let input = wrap(
        "#1=IFCRELDEFINESBYPROPERTIES(.NOTDEFINED.,IFCLABEL('a'),.A_VERY_LONG_ENUMERATION_VALUE.);\n\
         #2=ifcreldefinesbyproperties(.notdefined.,IFCLABEL('b'),.a_very_long_enumeration_value.);\n\
         #3=(IFCRELDEFINESBYPROPERTIES(.NOTDEFINED.)IFCLABEL());\n",
    );
    let exchange = parse(&input).expect("parses");
    let records = &exchange.data.records;
    let rel = |i: usize| &records[i].records()[0];
    let parameter = |i: usize, p: usize| match &rel(i).parameters[p] {
        Parameter::Enum(value)
        | Parameter::Typed {
            type_name: value, ..
        } => value.clone(),
        other => panic!("{other:?}"),
    };
    // Upper-cased as before.
    assert_eq!(rel(1).name, "IFCRELDEFINESBYPROPERTIES");
    assert_eq!(parameter(1, 2), "A_VERY_LONG_ENUMERATION_VALUE");
    // A long name is one allocation per spelling, shared by every use.
    assert!(rel(0).name.is_shared());
    assert!(rel(0).name.ptr_eq(&rel(2).name));
    // Spellings intern separately: equal text, two allocations.
    assert!(!parameter(0, 2).ptr_eq(&parameter(1, 2)));
    assert!(parameter(1, 2).is_shared());
    assert_eq!(rel(0).name, rel(1).name);
    // Short names and values are inline: nothing to allocate or share.
    assert!(!parameter(0, 0).is_shared());
    assert!(!parameter(0, 1).is_shared());
    assert!(!records[2].records()[1].name.is_shared());
    assert_eq!(parameter(0, 1), "IFCLABEL");
}

#[test]
fn str_behaves_like_the_text_it_holds() {
    let short = Str::from("abc");
    let long = Str::from("a".repeat(40));
    assert_eq!(short, "abc");
    assert_eq!(&*long, "a".repeat(40));
    assert_eq!(
        format!("{short:?} {long}"),
        format!("{:?} {}", "abc", "a".repeat(40))
    );
    assert!(!short.is_shared() && long.is_shared());
    assert!(long.clone().ptr_eq(&long));
    assert_eq!(Str::from(String::from("abc")), short);
    assert_eq!(Str::from(std::sync::Arc::<str>::from("abc")), short);
    assert!(!Str::from("x".repeat(22)).is_shared());
    assert!(Str::from("x".repeat(23)).is_shared());
    assert!(Str::from("é".repeat(11)).as_str().chars().count() == 11);
    assert_eq!(short.cmp(&Str::from("abd")), std::cmp::Ordering::Less);
}

#[test]
fn every_parse_path_builds_the_same_model() {
    let mut data = String::new();
    for id in 1..=3_000 {
        let _ = std::fmt::Write::write_fmt(
            &mut data,
            format_args!("#{id}=IFCPOINT((0.,{id}.,1.5E-3),.T.,IFCREAL(2.));\n"),
        );
    }
    let input = wrap(&data);
    let sequential = parse(&input).expect("parses");
    let parallel = parse_parallel_with(&input, ParseOptions::strict(), 8).expect("parses");
    assert_eq!(parallel.exchange, sequential);
    let scanned = scan(&input).expect("header");
    let decoded: Vec<DataRecord> = scanned
        .records()
        .map(|record| decode_record(&input, record.expect("scans").span).expect("decodes"))
        .collect();
    assert_eq!(decoded, sequential.data.records);
}

#[test]
fn constructors_accept_vectors_and_boxed_slices() {
    let simple: DataRecord = DataRecord::simple(
        InstanceId::from(1_u64),
        Str::from("A"),
        vec![Parameter::Null],
    );
    let complex: DataRecord = DataRecord::complex(
        InstanceId::from(2_u64),
        vec![Record::new(Str::from("A"), [Parameter::Null])],
    );
    assert_eq!(simple.records(), complex.records());
    assert_ne!(simple.instance, complex.instance);
}
