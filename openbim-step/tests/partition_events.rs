#![allow(missing_docs)]

use openbim_step::{
    data_record_spans, parse_events, partition_data_records, Event, EventSink, InstanceId,
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
        assert!(!local.is_empty());
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
            Event::HeaderRecord(record) => self.header_names.push(record.name),
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
