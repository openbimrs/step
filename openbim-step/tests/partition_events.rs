#![allow(missing_docs)]

use std::borrow::Cow;

use openbim_step::{
    data_record_spans, parse_events, parse_events_borrowed, parse_events_with,
    partition_data_records, DataRecord, Event, EventSink, InstanceId, ParseOptions, Span,
};

const INPUT: &[u8] = br#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('x'),'2;1');
FILE_NAME('x','',(),(),'','','');
FILE_SCHEMA(('S'));
ENDSEC;
DATA;
#1=A('text ; #999= NOPE');
#2=B((1,2),/* ; #888= NOPE */#1);
#3=C("0AFF");
#4=D($);
ENDSEC;
END-ISO-10303-21;
"#;

#[test]
fn partitions_are_nonempty_record_aligned_and_complete() {
    let spans = data_record_spans(INPUT).unwrap();
    assert_eq!(spans.len(), 4);

    let partitions = partition_data_records(INPUT, 3).unwrap();
    assert_eq!(partitions.len(), 3);
    assert_eq!(partitions.first().unwrap().start, spans[0].start);
    assert_eq!(partitions.last().unwrap().end, spans[3].end);
    for pair in partitions.windows(2) {
        assert_eq!(pair[0].end, pair[1].start);
    }

    let mut ids = Vec::new();
    for partition in &partitions {
        let local = data_record_spans(&INPUT[partition.start..partition.end]).unwrap();
        assert_ne!(local, [] as [openbim_step::Span; 0]);
        for span in local {
            let record = &INPUT[partition.start + span.start..partition.start + span.end];
            let hash = record.iter().position(|byte| *byte == b'#').unwrap();
            let equals = record.iter().position(|byte| *byte == b'=').unwrap();
            ids.push(
                std::str::from_utf8(&record[hash + 1..equals])
                    .unwrap()
                    .parse::<u64>()
                    .unwrap(),
            );
        }
    }
    assert_eq!(ids, vec![1, 2, 3, 4]);
}

#[test]
fn requesting_zero_partitions_is_rejected() {
    assert!(partition_data_records(INPUT, 0).is_err());
}

#[test]
fn record_only_partition_does_not_confuse_entity_names_with_sections() {
    let input = b"#1=DATA($);\n#2=ENDSEC($);";
    let spans = data_record_spans(input).unwrap();
    assert_eq!(spans.len(), 2);
    assert_eq!(&input[spans[0].start..spans[0].end], b"#1=DATA($);");
    assert_eq!(&input[spans[1].start..spans[1].end], b"#2=ENDSEC($);");
}

#[derive(Default)]
struct Collector {
    data_ids: Vec<InstanceId>,
    header_names: Vec<String>,
    transitions: Vec<&'static str>,
}

impl EventSink for Collector {
    fn event(&mut self, event: Event) {
        match event {
            Event::StartHeader => self.transitions.push("start-header"),
            Event::HeaderRecord(record) => self.header_names.push(record.name.to_string()),
            Event::EndHeader => self.transitions.push("end-header"),
            Event::StartData => self.transitions.push("start-data"),
            Event::DataRecord(record) => self.data_ids.push(record.id),
            Event::EndData => self.transitions.push("end-data"),
        }
    }
}

#[test]
fn event_sink_observes_prefix_before_a_late_lexical_error() {
    let input = br"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('x'),'2;1');
FILE_NAME('x','',(),(),'','','');
FILE_SCHEMA(('S'));
ENDSEC;
DATA;
#1=A($);
#2=B('unterminated
";
    let mut sink = Collector::default();
    assert!(parse_events(input, &mut sink).is_err());
    assert_eq!(sink.data_ids, [InstanceId::from(1_u64)]);
    assert_eq!(
        sink.transitions,
        ["start-header", "end-header", "start-data"]
    );
}

#[test]
fn event_sink_receives_records_in_source_order_without_building_exchange() {
    let mut sink = Collector::default();
    parse_events(INPUT, &mut sink).unwrap();
    assert_eq!(
        sink.header_names,
        ["FILE_DESCRIPTION", "FILE_NAME", "FILE_SCHEMA"]
    );
    assert_eq!(
        sink.data_ids,
        [
            InstanceId::from(1_u64),
            InstanceId::from(2_u64),
            InstanceId::from(3_u64),
            InstanceId::from(4_u64),
        ]
    );
    assert_eq!(
        sink.transitions,
        ["start-header", "end-header", "start-data", "end-data"]
    );
}

/// Keeps what the span-carrying method receives, and counts data records that
/// still arrive through `event`.
#[derive(Default)]
struct Positioned {
    spans: Vec<(InstanceId, Span)>,
    forwarded: usize,
}

impl EventSink for Positioned {
    fn event(&mut self, event: Event) {
        if matches!(event, Event::DataRecord(_)) {
            self.forwarded += 1;
        }
    }

    fn data_record(&mut self, record: DataRecord, span: Span) {
        self.spans.push((record.id, span));
    }
}

#[test]
fn a_sink_receives_each_data_record_with_the_span_of_its_bytes() {
    let mut sink = Positioned::default();
    parse_events(INPUT, &mut sink).unwrap();
    assert_eq!(sink.forwarded, 0, "the override replaces the event");
    let spans: Vec<Span> = sink.spans.iter().map(|(_, span)| *span).collect();
    assert_eq!(spans, data_record_spans(INPUT).unwrap());
    let (id, span) = &sink.spans[0];
    assert_eq!(*id, InstanceId::from(1_u64));
    assert_eq!(&INPUT[span.start..span.end], b"#1=A('text ; #999= NOPE');");
}

/// The positions stay those of the original input past a recovered record,
/// the case the method exists for.
#[test]
fn record_spans_stay_exact_after_a_skipped_record() {
    let input = b"ISO-10303-21;HEADER;FILE_DESCRIPTION((''),'2;1');\
        FILE_NAME('','',(),(),'','','');FILE_SCHEMA(('S'));ENDSEC;DATA;\n\
        #1=A(@);\n#2 = /* c */ B(1) ;\n#3=C(,);\n#4=D();\nENDSEC;END-ISO-10303-21;\n";
    let mut sink = Positioned::default();
    let diagnostics = parse_events_with(input, &mut sink, ParseOptions::lenient()).unwrap();
    assert_eq!(diagnostics.len(), 2);
    let kept: Vec<&[u8]> = sink
        .spans
        .iter()
        .map(|(_, span)| &input[span.start..span.end])
        .collect();
    assert_eq!(kept, [&b"#2 = /* c */ B(1) ;"[..], b"#4=D();"]);
}

/// A sink that only implements `event` keeps receiving every record, whether
/// it is a type, a closure, or borrows its text.
#[test]
fn the_default_forwards_data_records_to_event() {
    let mut typed = Collector::default();
    parse_events(INPUT, &mut typed).unwrap();
    assert_eq!(typed.data_ids.len(), 4);

    let mut owned = 0;
    parse_events(INPUT, &mut |event: Event| {
        owned += usize::from(matches!(event, Event::DataRecord(_)));
    })
    .unwrap();
    assert_eq!(owned, 4);

    let mut borrowed = 0;
    parse_events_borrowed(
        INPUT,
        &mut |event: Event<Cow<'_, str>>| {
            borrowed += usize::from(matches!(event, Event::DataRecord(_)));
        },
        ParseOptions::strict(),
    )
    .unwrap();
    assert_eq!(borrowed, 4);
}

/// The method is generic over the text type, like the trait.
#[test]
fn a_borrowing_sink_receives_the_same_spans() {
    struct Spans(Vec<Span>);
    impl<'a> EventSink<Cow<'a, str>> for Spans {
        fn event(&mut self, _: Event<Cow<'a, str>>) {}
        fn data_record(&mut self, _: DataRecord<Cow<'a, str>>, span: Span) {
            self.0.push(span);
        }
    }
    let mut sink = Spans(Vec::new());
    parse_events_borrowed(INPUT, &mut sink, ParseOptions::strict()).unwrap();
    assert_eq!(sink.0, data_record_spans(INPUT).unwrap());
}
