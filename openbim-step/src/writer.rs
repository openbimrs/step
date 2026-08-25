//! Semantic exchange writer.
//!
//! Output is structurally equivalent rather than byte-identical: whitespace,
//! comments, and keyword case are normalized while numeric lexemes are preserved.

use crate::escape;
use crate::lexer::{Lexer, Token};
use crate::{Exchange, HeaderRecord, Parameter, Record};
use std::io::{self, Write};

/// Writes a complete physical file.
///
/// # Errors
///
/// Returns an I/O error from `output`, or [`std::io::ErrorKind::InvalidInput`]
/// when an exchange contains an invalid identifier, number, binary, or empty instance.
pub fn write<S: AsRef<str>, W: Write + ?Sized>(
    exchange: &Exchange<S>,
    output: &mut W,
) -> io::Result<()> {
    validate_header(&exchange.header.records)?;
    writeln!(output, "ISO-10303-21;")?;
    writeln!(output, "HEADER;")?;
    for record in &exchange.header.records {
        write_identifier(record.name.as_ref(), output)?;
        write!(output, "(")?;
        write_parameters(&record.parameters, output, 0)?;
        writeln!(output, ");")?;
    }
    writeln!(output, "ENDSEC;")?;
    writeln!(output, "DATA;")?;
    for instance in &exchange.data.records {
        write!(output, "#{}=", instance.id.as_str())?;
        match instance.records.as_slice() {
            [] => return Err(invalid("data instance must contain a record")),
            [record] => write_record(record, output)?,
            records => {
                write!(output, "(")?;
                for record in records {
                    write_record(record, output)?;
                }
                write!(output, ")")?;
            }
        }
        writeln!(output, ";")?;
    }
    writeln!(output, "ENDSEC;")?;
    writeln!(output, "END-ISO-10303-21;")
}

/// Writes an exchange to an owned UTF-8 string.
///
/// The physical syntax emitted by this crate is ASCII; text content is escaped.
///
/// # Errors
///
/// Returns [`std::io::ErrorKind::InvalidInput`] for invalid STEP syntax values.
pub fn write_to_string<S: AsRef<str>>(exchange: &Exchange<S>) -> io::Result<String> {
    let mut bytes = Vec::new();
    write(exchange, &mut bytes)?;
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn invalid(detail: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail)
}

fn validate_header<S: AsRef<str>>(records: &[HeaderRecord<S>]) -> io::Result<()> {
    const REQUIRED: [&str; 3] = ["FILE_DESCRIPTION", "FILE_NAME", "FILE_SCHEMA"];
    if records.len() < REQUIRED.len() {
        return Err(invalid("missing mandatory STEP header record"));
    }
    for (record, required) in records.iter().zip(REQUIRED) {
        if !record.name.as_ref().eq_ignore_ascii_case(required) {
            return Err(invalid("mandatory STEP header records are out of order"));
        }
    }
    if records[REQUIRED.len()..].iter().any(|record| {
        REQUIRED
            .iter()
            .any(|required| record.name.as_ref().eq_ignore_ascii_case(required))
    }) {
        return Err(invalid("duplicate mandatory STEP header record"));
    }
    Ok(())
}

fn require_identifier(value: &str) -> io::Result<()> {
    let keyword = value.strip_prefix('!').unwrap_or(value);
    let mut bytes = keyword.bytes();
    let valid = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(invalid("invalid STEP identifier"))
    }
}

fn write_identifier<W: Write + ?Sized>(value: &str, output: &mut W) -> io::Result<()> {
    require_identifier(value)?;
    output.write_all(value.to_ascii_uppercase().as_bytes())
}

fn write_enumeration<W: Write + ?Sized>(value: &str, output: &mut W) -> io::Result<()> {
    if value.starts_with('!') {
        return Err(invalid("invalid STEP enumeration"));
    }
    write_identifier(value, output)
}

fn require_number(value: &str, real: bool) -> io::Result<()> {
    let mut lexer = Lexer::new(value.as_bytes());
    let valid = lexer.next().is_some_and(|token| {
        token.is_ok_and(|token| {
            token.span.start == 0
                && token.span.end == value.len()
                && if real {
                    matches!(token.value, Token::Real(_))
                } else {
                    matches!(token.value, Token::Integer(_))
                }
        })
    }) && lexer.next().is_none();
    if valid {
        Ok(())
    } else {
        Err(invalid("invalid STEP number"))
    }
}

fn require_binary(value: &str) -> io::Result<()> {
    let bytes = value.as_bytes();
    if matches!(bytes.first(), Some(b'0'..=b'3')) && bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        Ok(())
    } else {
        Err(invalid("invalid STEP binary"))
    }
}

fn write_record<S: AsRef<str>, W: Write + ?Sized>(
    record: &Record<S>,
    output: &mut W,
) -> io::Result<()> {
    write_identifier(record.name.as_ref(), output)?;
    write!(output, "(")?;
    write_parameters(&record.parameters, output, 0)?;
    write!(output, ")")
}

fn write_parameters<S: AsRef<str>, W: Write + ?Sized>(
    parameters: &[Parameter<S>],
    output: &mut W,
    depth: usize,
) -> io::Result<()> {
    for (index, parameter) in parameters.iter().enumerate() {
        if index != 0 {
            write!(output, ",")?;
        }
        write_parameter_at(parameter, output, depth)?;
    }
    Ok(())
}

/// Writes one generic parameter.
///
/// # Errors
///
/// Returns an I/O error from `output`, or [`std::io::ErrorKind::InvalidInput`]
/// when `parameter` contains an invalid unescaped syntax value.
pub fn write_parameter<S: AsRef<str>, W: Write + ?Sized>(
    parameter: &Parameter<S>,
    output: &mut W,
) -> io::Result<()> {
    write_parameter_at(parameter, output, 0)
}

fn write_parameter_at<S: AsRef<str>, W: Write + ?Sized>(
    parameter: &Parameter<S>,
    output: &mut W,
    depth: usize,
) -> io::Result<()> {
    if depth > crate::MAX_PARAMETER_NESTING {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "parameter nesting limit exceeded",
        ));
    }
    match parameter {
        Parameter::Null => write!(output, "$"),
        Parameter::Derived => write!(output, "*"),
        Parameter::Bool(true) => write!(output, ".T."),
        Parameter::Bool(false) => write!(output, ".F."),
        Parameter::LogicalUnknown => write!(output, ".U."),
        Parameter::Integer(value) => {
            require_number(value.as_ref(), false)?;
            write!(output, "{}", value.as_ref())
        }
        Parameter::Real(value) => {
            require_number(value.as_ref(), true)?;
            write!(output, "{}", value.as_ref())
        }
        Parameter::Text(text) => write!(output, "'{}'", escape::encode(text.as_ref())),
        Parameter::Binary(binary) => {
            require_binary(binary.as_ref())?;
            write!(output, "\"{}\"", binary.as_ref())
        }
        Parameter::Enum(value) => {
            write!(output, ".")?;
            write_enumeration(value.as_ref(), output)?;
            write!(output, ".")
        }
        Parameter::Ref(id) => write!(output, "#{id_value}", id_value = id.as_str()),
        Parameter::List(items) => {
            write!(output, "(")?;
            write_parameters(items, output, depth + 1)?;
            write!(output, ")")
        }
        Parameter::Typed { type_name, value } => {
            write_identifier(type_name.as_ref(), output)?;
            write!(output, "(")?;
            write_parameter_at(value, output, depth + 1)?;
            write!(output, ")")
        }
    }
}
