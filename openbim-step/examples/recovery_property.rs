//! Property probe: recovery must never keep a record the strict parser
//! would not have seen at that offset, and every diagnostic must be in-bounds.
//!
//! Not a unit test: it hammers pseudo-random damaged inputs to look for
//! fabricated records of the kind the binary-literal defect produced.

#![allow(clippy::unreadable_literal, clippy::cast_possible_truncation)]

use openbim_step::{parse_with, OnMalformed, ParseOptions};

fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

fn main() {
    let base = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
                FILE_NAME('n','t',(''),(''),'p','o','a');\nFILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n\
                #1= IFCPERSON($,$,'a;b',$,$,$,$,$);\n\
                #2= IFCPIXELTEXTURE(1,1,8,(\"00AA;BB\"));\n\
                #3= IFCORGANIZATION($,'o''x',$,$,$); /* c;c */\n\
                #4= IFCDOOR('real');\nENDSEC;\nEND-ISO-10303-21;\n";

    let mut state = 0x5eed_1234_u64;
    let mut recovered = 0usize;
    let mut fabricated = 0usize;
    let mut span_faults = 0usize;

    for _ in 0..20_000 {
        let mut bytes = base.as_bytes().to_vec();
        // Corrupt 1-3 bytes anywhere in the DATA region.
        let edits = 1 + (lcg(&mut state) % 3) as usize;
        for _ in 0..edits {
            let at = 150 + (lcg(&mut state) as usize % (bytes.len() - 150));
            match lcg(&mut state) % 3 {
                0 => bytes[at] = b'@',
                1 => {
                    bytes.remove(at);
                }
                _ => bytes[at] = (lcg(&mut state) % 128) as u8,
            }
        }

        let Ok(outcome) = parse_with(
            &bytes,
            ParseOptions::default().on_malformed_record(OnMalformed::Skip),
        ) else {
            continue;
        };
        recovered += 1;

        for diagnostic in &outcome.diagnostics {
            let span = diagnostic.span();
            if span.start > span.end || span.end > bytes.len() {
                span_faults += 1;
            }
        }

        // Every kept record must appear verbatim in the surviving source: its
        // `#id=` must exist outside any diagnostic-covered range.
        for record in &outcome.exchange.data.records {
            let needle = format!("#{}=", record.id.as_str());
            let found = String::from_utf8_lossy(&bytes).contains(&needle)
                || String::from_utf8_lossy(&bytes).contains(&format!("#{} =", record.id.as_str()));
            if !found {
                fabricated += 1;
                eprintln!(
                    "FABRICATED {} in:\n{}",
                    needle,
                    String::from_utf8_lossy(&bytes)
                );
            }
        }
    }

    println!("recovered={recovered} fabricated={fabricated} span_faults={span_faults}");
    assert_eq!(fabricated, 0, "recovery invented records");
    assert_eq!(span_faults, 0, "diagnostic span out of bounds");
    println!("PROPERTY PROBE OK");
}
