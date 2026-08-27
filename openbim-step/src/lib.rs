//! Generic ISO 10303 STEP Part 21 and EXPRESS syntax infrastructure.
//!
//! The crate has no domain-schema dependency. It tokenizes bytes with spans,
//! decodes physical-file strings, parses records into generic parameters,
//! writes them back, partitions complete records, and offers a structural,
//! explicitly partial EXPRESS declaration extractor.

#![forbid(unsafe_code)]

mod diagnostic;
/// STEP string escape codec.
pub mod escape;
/// Structural, explicitly partial EXPRESS declaration extraction.
pub mod express;
mod header;
/// Physical-file tokenizer.
pub mod lexer;
mod model;
mod parser;
mod partition;
/// Malformed-record recovery policy and non-fatal diagnostics.
pub mod recovery;
/// Semantic exchange writer.
pub mod writer;

pub use diagnostic::{Source, SourceLocation, Span, Spanned, StepError};
pub use header::is_step_file;
pub use model::{
    DataRecord, DataSection, Exchange, HeaderRecord, HeaderSection, InstanceId, Parameter, Record,
    StandardHeader,
};
pub use parser::{parse, parse_events, parse_events_with, parse_with, Event, EventSink};
pub use partition::{data_record_spans, partition_data_records, Partition};
pub use recovery::{Diagnostic, OnMalformed, ParseOptions, ParseOutcome, Severity};
pub use writer::{write, write_parameter, write_to_string};

/// Maximum nesting accepted for lists and typed parameters.
pub const MAX_PARAMETER_NESTING: usize = 128;
