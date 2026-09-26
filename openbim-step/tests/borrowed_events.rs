#![allow(missing_docs)]
//! `parse_events_borrowed`: same events as `parse_events_with`, text borrowed
//! from the input where it needs no rewriting, names in source case.

use std::borrow::Cow;

use openbim_step::{
    parse_events_borrowed, parse_events_with, Event, Instance, Parameter, ParseOptions, Str,
};

const FILE: &[u8] = b"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('d'),'2;1');
FILE_NAME('n','t',('a'),('o'),'p','s','z');
FILE_SCHEMA(('IFC4'));
ENDSEC;
DATA;
#1=IfcWall('plain','it''s','\\X2\\00E4\\X0\\',.notDefined.,IfcLabel('x'),12,-1.5E3,\"0F\",$,*,(#2,#3));
#2=IFCW\nALL(1\t2,'a\nb');
ENDSEC;
END-ISO-10303-21;
";

fn borrowed(input: &[u8]) -> Vec<Event<Cow<'_, str>>> {
    let mut events = Vec::new();
    parse_events_borrowed(
        input,
        &mut |event| events.push(event),
        ParseOptions::strict(),
    )
    .expect("parses");
    events
}

fn data<'e, 'a>(
    events: &'e [Event<Cow<'a, str>>],
) -> Vec<&'e openbim_step::DataRecord<Cow<'a, str>>> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::DataRecord(record) => Some(record),
            _ => None,
        })
        .collect()
}

/// The value as `&str` if it is borrowed from the input, else `None`.
// `&Cow` on purpose: the variant is what these helpers inspect.
#[allow(clippy::ptr_arg)]
fn borrowed_str<'v>(value: &'v Cow<'_, str>) -> Option<&'v str> {
    match value {
        Cow::Borrowed(text) => Some(text),
        Cow::Owned(_) => None,
    }
}

/// The value as `&str` if it had to be owned (rewritten), else `None`.
#[allow(clippy::ptr_arg)]
fn owned_str<'v>(value: &'v Cow<'_, str>) -> Option<&'v str> {
    match value {
        Cow::Owned(text) => Some(text),
        Cow::Borrowed(_) => None,
    }
}

#[test]
fn clean_values_are_borrowed_in_source_case() {
    let events = borrowed(FILE);
    let records = data(&events);
    let wall = &records[0].records()[0];
    assert_eq!(borrowed_str(&wall.name), Some("IfcWall"));
    let p = &wall.parameters;
    assert!(matches!(&p[0], Parameter::Text(t) if borrowed_str(t) == Some("plain")));
    assert!(matches!(&p[3], Parameter::Enum(e) if borrowed_str(e) == Some("notDefined")));
    assert!(matches!(&p[4], Parameter::Typed { type_name, value }
        if borrowed_str(type_name) == Some("IfcLabel")
            && matches!(value.as_ref(), Parameter::Text(t) if borrowed_str(t) == Some("x"))));
    assert!(matches!(&p[5], Parameter::Integer(v) if borrowed_str(v) == Some("12")));
    assert!(matches!(&p[6], Parameter::Real(v) if borrowed_str(v) == Some("-1.5E3")));
    assert!(matches!(&p[7], Parameter::Binary(v) if borrowed_str(v) == Some("0F")));
    assert_eq!(p[8], Parameter::Null);
    assert_eq!(p[9], Parameter::Derived);
}

#[test]
fn text_that_needs_decoding_is_owned_and_decoded() {
    let events = borrowed(FILE);
    let p = &data(&events)[0].records()[0].parameters;
    assert!(matches!(&p[1], Parameter::Text(t) if owned_str(t) == Some("it's")));
    assert!(matches!(&p[2], Parameter::Text(t) if owned_str(t) == Some("\u{e4}")));
}

#[test]
fn values_interrupted_by_ignored_controls_are_owned_and_stripped() {
    let events = borrowed(FILE);
    let second = &data(&events)[1].records()[0];
    assert_eq!(owned_str(&second.name), Some("IFCWALL"));
    assert!(matches!(&second.parameters[0], Parameter::Integer(v) if owned_str(v) == Some("12")));
    // A control inside a string body is dropped from the lexeme, so the
    // body needs rewriting and cannot be borrowed.
    assert!(matches!(&second.parameters[1], Parameter::Text(t) if owned_str(t) == Some("ab")));
}

/// The borrowed stream, normalised (names upper-cased, owned strings), is
/// exactly the owned stream -- events, diagnostics and errors -- for good,
/// damaged and recovering inputs.
#[test]
fn borrowed_events_normalise_to_the_owned_events() {
    fn owned(text: Cow<'_, str>) -> Str {
        Str::from(text.into_owned())
    }
    fn upper(text: &str) -> Str {
        Str::from(text.to_ascii_uppercase())
    }
    fn param(p: Parameter<Cow<'_, str>>) -> Parameter {
        match p {
            Parameter::Null => Parameter::Null,
            Parameter::Derived => Parameter::Derived,
            Parameter::Bool(b) => Parameter::Bool(b),
            Parameter::LogicalUnknown => Parameter::LogicalUnknown,
            Parameter::Integer(v) => Parameter::Integer(owned(v)),
            Parameter::Real(v) => Parameter::Real(owned(v)),
            Parameter::Text(v) => Parameter::Text(owned(v)),
            Parameter::Binary(v) => Parameter::Binary(owned(v)),
            Parameter::Enum(v) => Parameter::Enum(upper(&v)),
            Parameter::Ref(id) => Parameter::Ref(id),
            Parameter::List(items) => {
                Parameter::List(items.into_vec().into_iter().map(param).collect())
            }
            Parameter::Typed { type_name, value } => Parameter::Typed {
                type_name: upper(&type_name),
                value: Box::new(param(*value)),
            },
        }
    }
    fn event(e: Event<Cow<'_, str>>) -> Event {
        match e {
            Event::StartHeader => Event::StartHeader,
            Event::EndHeader => Event::EndHeader,
            Event::StartData => Event::StartData,
            Event::EndData => Event::EndData,
            Event::HeaderRecord(h) => Event::HeaderRecord(openbim_step::HeaderRecord {
                name: upper(&h.name),
                parameters: h.parameters.into_vec().into_iter().map(param).collect(),
            }),
            Event::DataRecord(d) => {
                let record = |r: openbim_step::Record<Cow<'_, str>>| {
                    openbim_step::Record::new(
                        upper(&r.name),
                        r.parameters
                            .into_vec()
                            .into_iter()
                            .map(param)
                            .collect::<Vec<_>>(),
                    )
                };
                Event::DataRecord(openbim_step::DataRecord {
                    id: d.id,
                    instance: match d.instance {
                        Instance::Simple(r) => Instance::Simple(record(r)),
                        Instance::Complex(rs) => {
                            Instance::Complex(rs.into_vec().into_iter().map(record).collect())
                        }
                    },
                })
            }
        }
    }
    let damaged = b"ISO-10303-21;HEADER;FILE_DESCRIPTION((''),'2;1');\
FILE_NAME('','',(''),(''),'','','');FILE_SCHEMA(('X'));ENDSEC;\
DATA;#1=a(1);#2=b(,);#3=c(#9);ENDSEC;END-ISO-10303-21;";
    for input in [FILE, &damaged[..], b"ISO-10303-21;HEADER;ENDSEC;"] {
        for options in [
            ParseOptions::strict(),
            ParseOptions::lenient().check_references(true),
        ] {
            let mut owned = Vec::new();
            let owned_result = parse_events_with(input, &mut |e: Event| owned.push(e), options);
            let mut normalised = Vec::new();
            let borrowed_result = parse_events_borrowed(
                input,
                &mut |e: Event<Cow<'_, str>>| normalised.push(event(e)),
                options,
            );
            assert_eq!(format!("{owned_result:?}"), format!("{borrowed_result:?}"));
            assert_eq!(owned, normalised);
        }
    }
}
