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

    for malformed in [b"!_BAD".as_slice(), b"!BAD_NAME"] {
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
fn lexer_rejects_keywords_with_a_leading_underscore() {
    assert!(Lexer::new(b"_BAD").collect::<Result<Vec<_>, _>>().is_err());
}

#[test]
fn line_delimiters_are_ignored_even_inside_tokens() {
    let tokens = Lexer::new(b"/\n* c *\r/ #1\n2=ENT\rITY('it'\n's',1.\t0);")
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
