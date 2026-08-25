#![allow(missing_docs)]

use openbim_step::escape::{decode, encode};
use openbim_step::lexer::{Lexer, Token};
use openbim_step::Span;

#[test]
fn tokenizer_reports_spans_and_handles_step_lexemes() {
    let input = b" /* c */ #12=THING('it''s',1.E3,-0.5,.U.,\"0AF\");";
    let tokens = Lexer::new(input)
        .collect::<Result<Vec<_>, _>>()
        .expect("lexing succeeds");
    assert!(matches!(&tokens[0].value, Token::Id(value) if value.as_ref() == b"12"));
    assert_eq!(tokens[0].span, Span::new(9, 12));
    assert!(tokens
        .iter()
        .any(|t| matches!(&t.value, Token::Real(value) if value.as_ref() == b"1.E3")));
    assert!(tokens
        .iter()
        .any(|t| matches!(&t.value, Token::Real(value) if value.as_ref() == b"-0.5")));
    assert!(tokens
        .iter()
        .any(|t| matches!(&t.value, Token::Text(value) if value.as_ref() == b"it''s")));
    assert!(tokens
        .iter()
        .any(|t| matches!(&t.value, Token::Binary(value) if value.as_ref() == b"0AF")));
}

#[test]
fn lexer_preserves_arbitrary_precision_numbers_and_rejects_bad_binary() {
    let tokens = Lexer::new(b"123456789012345678901234567890 1.E+999")
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(matches!(
        &tokens[0].value,
        Token::Integer(value) if value.as_ref() == b"123456789012345678901234567890"
    ));
    assert!(matches!(
        &tokens[1].value,
        Token::Real(value) if value.as_ref() == b"1.E+999"
    ));

    for malformed in [b"\"\"".as_slice(), b"\"4FF\"", b"\"0FG\""] {
        assert!(Lexer::new(malformed).next().unwrap().is_err());
    }
}

#[test]
fn lexer_rejects_reals_without_a_significand() {
    for malformed in [b"-E2".as_slice(), b"+E-3", b"-.5", b".5", b"1E3"] {
        assert!(
            Lexer::new(malformed)
                .collect::<Result<Vec<_>, _>>()
                .is_err(),
            "accepted {}",
            String::from_utf8_lossy(malformed)
        );
    }
}

#[test]
fn lexer_accepts_user_defined_keywords() {
    let tokens = Lexer::new(b"!VENDOR !WRAPPED")
        .collect::<Result<Vec<_>, _>>()
        .expect("user-defined keyword lexes");
    assert!(matches!(&tokens[0].value, Token::Name(value) if value.as_ref() == b"!VENDOR"));
    assert!(matches!(&tokens[1].value, Token::Name(value) if value.as_ref() == b"!WRAPPED"));

    for malformed in [b"!".as_slice(), b"!9BAD", b"!BAD-NAME"] {
        assert!(
            Lexer::new(malformed)
                .collect::<Result<Vec<_>, _>>()
                .is_err(),
            "accepted {}",
            String::from_utf8_lossy(malformed)
        );
    }
}

#[test]
fn lexer_accepts_low_line_as_an_upper_character() {
    let tokens = Lexer::new(b"_ENTITY !VENDOR_EXT ._ENUM.")
        .collect::<Result<Vec<_>, _>>()
        .expect("low lines are valid UPPER characters in Part 21 keywords");
    assert!(matches!(&tokens[0].value, Token::Name(value) if value.as_ref() == b"_ENTITY"));
    assert!(matches!(&tokens[1].value, Token::Name(value) if value.as_ref() == b"!VENDOR_EXT"));
    assert!(matches!(&tokens[2].value, Token::Keyword(value) if value.as_ref() == b"_ENUM"));
}

#[test]
fn print_directives_are_ignored_inside_wide_text_and_quote_doubling() {
    let tokens = Lexer::new(br"'\X2\0041\N\0042\X0\' 'a'\F\'b'")
        .collect::<Result<Vec<_>, _>>()
        .expect("print directives are insignificant inside strings");
    assert!(
        matches!(&tokens[0].value, Token::Text(value) if value.as_ref() == br"\X2\00410042\X0\")
    );
    assert!(matches!(&tokens[1].value, Token::Text(value) if value.as_ref() == b"a''b"));
    assert_eq!(
        decode(match &tokens[0].value {
            Token::Text(value) => value,
            _ => unreachable!(),
        }),
        "AB"
    );
    assert_eq!(
        decode(match &tokens[1].value {
            Token::Text(value) => value,
            _ => unreachable!(),
        }),
        "a'b"
    );
}

#[test]
fn escaped_literal_print_directive_text_is_preserved() {
    let tokens = Lexer::new(br"'\\N\\' '\\F\\'")
        .collect::<Result<Vec<_>, _>>()
        .expect("escaped reverse solidi are text, not print directives");
    assert_eq!(
        decode(match &tokens[0].value {
            Token::Text(value) => value,
            _ => unreachable!(),
        }),
        "\\N\\"
    );
    assert_eq!(
        decode(match &tokens[1].value {
            Token::Text(value) => value,
            _ => unreachable!(),
        }),
        "\\F\\"
    );
}

#[test]
fn line_delimiters_are_ignored_even_inside_tokens() {
    let tokens = Lexer::new(b"\\N\\/\n* c *\r/ #1\n2=ENT\rITY('it'\n's',1.\t0,\"0A\\F\\B\");")
        .collect::<Result<Vec<_>, _>>()
        .expect("line delimiters are insignificant");
    assert!(matches!(&tokens[0].value, Token::Id(value) if value.as_ref() == b"12"));
    assert!(tokens
        .iter()
        .any(|token| matches!(&token.value, Token::Name(value) if value.as_ref() == b"ENTITY")));
    assert!(tokens
        .iter()
        .any(|token| matches!(&token.value, Token::Text(value) if value.as_ref() == b"it''s")));
    assert!(tokens
        .iter()
        .any(|token| matches!(&token.value, Token::Real(value) if value.as_ref() == b"1.0")));
    assert!(tokens
        .iter()
        .any(|token| matches!(&token.value, Token::Binary(value) if value.as_ref() == b"0AB")));
}

#[test]
fn unterminated_literals_are_diagnostics_not_silent_eof() {
    let error = Lexer::new(b"'never closed")
        .collect::<Result<Vec<_>, _>>()
        .unwrap_err();
    assert_eq!(error.span(), Span::new(0, 13));
    assert!(error.to_string().contains("unterminated string"));
}

#[test]
fn escape_codec_supports_all_part21_forms_and_roundtrips_unicode() {
    assert_eq!(decode(b"it''s"), "it's");
    assert_eq!(decode(b"\\S\\D"), "Ä");
    assert_eq!(decode(b"\\PB\\\\S\\!"), "Ą");
    assert_eq!(decode(b"\\X\\41"), "A");
    assert_eq!(decode(b"\\X2\\30D330EB\\X0\\"), "ビル");
    assert_eq!(decode(b"\\X4\\0001F642\\X0\\"), "🙂");
    assert_eq!(decode(b"A\\N\\B\\F\\C"), "ABC");
    assert_eq!(decode("Größe".as_bytes()), "Größe");
    assert_eq!(decode(b"\\Q\\x"), "\\Q\\x");
    assert_eq!(decode(b"\\X2\\D800\\X0\\"), "\\X2\\D800\\X0\\");
    assert_eq!(decode(b"\\X4\\00110000\\X0\\"), "\\X4\\00110000\\X0\\");
    assert_eq!(decode(b"\\X2\\\\X0\\"), "\\X2\\\\X0\\");

    assert!(encode("🙂").contains("\\X4\\0001F642\\X0\\"));

    for original in [
        "plain",
        "it's \\ safe",
        "line\nNUL\0end",
        "ÄÖÜ",
        "ビル",
        "🙂",
    ] {
        assert_eq!(decode(encode(original).as_bytes()), original);
    }
    assert!(!encode("line\nNUL\0end").contains(['\n', '\0']));
}
