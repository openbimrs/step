//! STEP string-literal escape decoding and encoding.
//!
//! Supported forms are doubled apostrophes, doubled backslashes, `\N\`, `\F\`,
//! `\S\`, `\X\`, UTF-16BE `\X2\...\X0\`, and UTF-32BE `\X4\...\X0\`.

/// Decodes a STEP string literal body to Unicode.
///
/// Unknown or malformed escapes are preserved verbatim instead of discarded.
#[must_use]
pub fn decode(raw: &[u8]) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut position = 0;
    let mut alphabet = b'A';
    while position < raw.len() {
        match raw[position] {
            b'\'' if raw.get(position + 1) == Some(&b'\'') => {
                output.push('\'');
                position += 2;
            }
            b'\'' => {
                output.push('\'');
                position += 1;
            }
            b'\\' => {
                if let Some(consumed) = decode_escape(&raw[position..], &mut output, &mut alphabet)
                {
                    position += consumed;
                } else {
                    output.push('\\');
                    position += 1;
                }
            }
            _ => {
                let start = position;
                while !matches!(raw.get(position), None | Some(b'\'' | b'\\')) {
                    position += 1;
                }
                output.push_str(&String::from_utf8_lossy(&raw[start..position]));
            }
        }
    }
    output
}

fn decode_escape(input: &[u8], output: &mut String, alphabet: &mut u8) -> Option<usize> {
    match input.get(1)? {
        b'\\' => {
            output.push('\\');
            Some(2)
        }
        b'N' | b'n' | b'F' | b'f' if input.get(2) == Some(&b'\\') => Some(3),
        b'S' | b's' if input.get(2) == Some(&b'\\') => {
            let byte = input.get(3)?.wrapping_add(128);
            output.push(decode_alphabet_byte(*alphabet, byte));
            Some(4)
        }
        b'P' | b'p'
            if matches!(input.get(2), Some(b'A'..=b'I' | b'a'..=b'i'))
                && input.get(3) == Some(&b'\\') =>
        {
            *alphabet = input[2].to_ascii_uppercase();
            Some(4)
        }
        b'X' | b'x' => match input.get(2)? {
            b'\\' => {
                let byte = u8::try_from(parse_hex(input.get(3..5)?)?).ok()?;
                output.push(char::from(byte));
                Some(5)
            }
            b'2' => {
                decode_wide(input, output, 4).or_else(|| preserve_malformed_wide(input, output))
            }
            b'4' => {
                decode_wide(input, output, 8).or_else(|| preserve_malformed_wide(input, output))
            }
            _ => None,
        },
        _ => None,
    }
}

fn decode_alphabet_byte(alphabet: u8, byte: u8) -> char {
    if alphabet == b'A' {
        return char::from(byte);
    }
    let encoding = match alphabet {
        b'B' => encoding_rs::ISO_8859_2,
        b'C' => encoding_rs::ISO_8859_3,
        b'D' => encoding_rs::ISO_8859_4,
        b'E' => encoding_rs::ISO_8859_5,
        b'F' => encoding_rs::ISO_8859_6,
        b'G' => encoding_rs::ISO_8859_7,
        b'H' => encoding_rs::ISO_8859_8,
        b'I' => encoding_rs::WINDOWS_1254,
        _ => encoding_rs::WINDOWS_1252,
    };
    let bytes = [byte];
    let (decoded, _, _) = encoding.decode(&bytes);
    decoded.chars().next().unwrap_or('\u{fffd}')
}

fn parse_hex(input: &[u8]) -> Option<u32> {
    u32::from_str_radix(std::str::from_utf8(input).ok()?, 16).ok()
}

fn preserve_malformed_wide(input: &[u8], output: &mut String) -> Option<usize> {
    let end = input
        .get(4..)?
        .windows(4)
        .position(|window| window.eq_ignore_ascii_case(b"\\X0\\"))?;
    let consumed = 4 + end + 4;
    output.push_str(&String::from_utf8_lossy(&input[..consumed]));
    Some(consumed)
}

fn decode_wide(input: &[u8], output: &mut String, digits: usize) -> Option<usize> {
    if input.get(3) != Some(&b'\\') {
        return None;
    }
    let mut position = 4;
    let mut utf16 = Vec::new();
    let mut utf32 = Vec::new();
    loop {
        if input
            .get(position..position + 4)?
            .eq_ignore_ascii_case(b"\\X0\\")
        {
            position += 4;
            break;
        }
        let unit = parse_hex(input.get(position..position + digits)?)?;
        if digits == 4 {
            utf16.push(u16::try_from(unit).ok()?);
        } else {
            utf32.push(unit);
        }
        position += digits;
    }
    let mut decoded = String::new();
    if digits == 4 {
        if utf16.is_empty() {
            return None;
        }
        for value in std::char::decode_utf16(utf16) {
            decoded.push(value.ok()?);
        }
    } else {
        if utf32.is_empty() {
            return None;
        }
        for value in utf32 {
            decoded.push(char::from_u32(value)?);
        }
    }
    output.push_str(&decoded);
    Some(position)
}

fn flush_utf16(pending: &mut Vec<u16>, output: &mut String) {
    if pending.is_empty() {
        return;
    }
    output.push_str("\\X2\\");
    for unit in pending.drain(..) {
        use std::fmt::Write as _;
        let _ = write!(output, "{unit:04X}");
    }
    output.push_str("\\X0\\");
}

/// Encodes Unicode as a STEP string literal body.
///
/// Printable ASCII is retained, apostrophes and backslashes are doubled, and
/// BMP runs use UTF-16BE `\X2\` notation and supplementary characters use
/// UTF-32BE `\X4\` notation.
#[must_use]
pub fn encode(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut pending = Vec::new();

    for character in text.chars() {
        match character {
            '\'' => {
                flush_utf16(&mut pending, &mut output);
                output.push_str("''");
            }
            '\\' => {
                flush_utf16(&mut pending, &mut output);
                output.push_str("\\\\");
            }
            value if value.is_ascii_control() => {
                use std::fmt::Write as _;

                flush_utf16(&mut pending, &mut output);
                let _ = write!(output, "\\X\\{:02X}", value as u32);
            }
            value if value.is_ascii() => {
                flush_utf16(&mut pending, &mut output);
                output.push(value);
            }
            value if u32::from(value) <= 0xffff => {
                let mut units = [0_u16; 2];
                pending.extend_from_slice(value.encode_utf16(&mut units));
            }
            value => {
                use std::fmt::Write as _;

                flush_utf16(&mut pending, &mut output);
                let _ = write!(output, "\\X4\\{:08X}\\X0\\", u32::from(value));
            }
        }
    }
    flush_utf16(&mut pending, &mut output);
    output
}
