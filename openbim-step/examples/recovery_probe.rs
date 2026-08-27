//! Probe: parse the real damaged AC20-FZK-Haus.ifc under both policies.

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let bytes = std::fs::read(&path).expect("read");

    match openbim_step::parse(&bytes) {
        Ok(_) => println!("strict: OK (unexpected)"),
        Err(error) => println!("strict: {error}"),
    }

    let outcome = openbim_step::parse_with(&bytes, openbim_step::ParseOptions::lenient())
        .expect("lenient parse");
    println!(
        "lenient: {} records, {} diagnostics",
        outcome.exchange.data.records.len(),
        outcome.diagnostics.len()
    );
    for diagnostic in &outcome.diagnostics {
        println!("  {diagnostic}");
        let span = diagnostic.span();
        println!(
            "  dropped bytes: {:?}",
            String::from_utf8_lossy(&bytes[span.start..span.end])
        );
    }
    let boundaries = outcome
        .exchange
        .data
        .records
        .iter()
        .filter(|record| {
            record
                .as_simple()
                .is_some_and(|simple| simple.name == "IFCRELSPACEBOUNDARY")
        })
        .count();
    println!("IfcRelSpaceBoundary: {boundaries}");
    println!("schema: {:?}", outcome.exchange.header.standard().schema);
}
