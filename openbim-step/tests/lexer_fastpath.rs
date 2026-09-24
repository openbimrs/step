#![allow(missing_docs)]
//! Lexer fast paths: ignored controls (ISO 10303-21:2016 §5.4) inside every
//! token kind, borrowed-versus-owned lexemes, and exact error spans. These
//! pin the behaviour the byte-at-a-time lexer had, so the fast paths cannot
//! drift from it.

use std::borrow::Cow;

use openbim_step::lexer::{Lexer, Token};
use openbim_step::Span;

fn tokens(input: &[u8]) -> Vec<(Token<'_>, Span)> {
    Lexer::new(input)
        .map(|token| {
            let token = token.expect("lexes");
            (token.value, token.span)
        })
        .collect()
}

fn owned(token: &Token<'_>) -> bool {
    matches!(
        token,
        Token::Id(Cow::Owned(_))
            | Token::Name(Cow::Owned(_))
            | Token::Integer(Cow::Owned(_))
            | Token::Real(Cow::Owned(_))
            | Token::Text(Cow::Owned(_))
    )
}

#[test]
fn every_control_is_dropped_from_every_position_in_a_number() {
    // TAB, LF, CR and FF after the sign, among the digits, around the point,
    // around the exponent marker and after the exponent sign.
    let input = b"-\t1\n2\r.\x0c5\tE\n-\r0\x0c7";
    let lexed = tokens(input);
    assert_eq!(lexed.len(), 1);
    assert!(matches!(&lexed[0].0, Token::Real(value) if value.as_ref() == b"-12.5E-07"));
    assert!(owned(&lexed[0].0));
    assert_eq!(lexed[0].1, Span::new(0, input.len()));
}

#[test]
fn clean_lexemes_borrow_and_dirty_ones_copy() {
    let lexed = tokens(b"#12=IFCWALL(-1.5E3,42);#1\n3=X\tY(4\r2);");
    let summary: Vec<(String, bool)> = lexed
        .iter()
        .filter_map(|(token, _)| match token {
            Token::Id(v) | Token::Name(v) | Token::Integer(v) | Token::Real(v) => {
                Some((String::from_utf8_lossy(v).into_owned(), owned(token)))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("12".to_owned(), false),
            ("IFCWALL".to_owned(), false),
            ("-1.5E3".to_owned(), false),
            ("42".to_owned(), false),
            ("13".to_owned(), true),
            ("XY".to_owned(), true),
            ("42".to_owned(), true),
        ]
    );
}

#[test]
fn a_control_before_an_id_is_trivia_not_part_of_the_lexeme() {
    // Controls between `#` and the first digit are skipped, and the digits
    // that follow are still borrowed: the lexeme itself is clean.
    let lexed = tokens(b"#\n\t7=A();");
    assert!(matches!(&lexed[0].0, Token::Id(Cow::Borrowed(value)) if *value == b"7"));
    assert_eq!(lexed[0].1, Span::new(0, 4));
}

#[test]
fn a_number_state_does_not_leak_into_the_next_number() {
    // The first number sees a control; the second must still borrow.
    let lexed = tokens(b"(1\n2,34)");
    let numbers: Vec<bool> = lexed
        .iter()
        .filter(|(token, _)| matches!(token, Token::Integer(_)))
        .map(|(token, _)| owned(token))
        .collect();
    assert_eq!(numbers, [true, false]);
}

#[test]
fn a_directive_or_comment_is_still_found_after_whitespace() {
    let lexed = tokens(b"  \\N\\ /* c */\n\\F\\#1=A();");
    assert!(matches!(&lexed[0].0, Token::Id(value) if value.as_ref() == b"1"));
    // `  \N\ /* c */` + LF + `\F\` is 17 bytes; the id starts right after.
    assert_eq!(lexed[0].1, Span::new(17, 19));
}

#[test]
fn a_slash_that_is_not_a_comment_is_a_syntax_error_at_the_slash() {
    let error = Lexer::new(b"  /x")
        .collect::<Result<Vec<_>, _>>()
        .expect_err("a lone slash is not a token");
    assert_eq!(error.span(), Span::new(2, 3));
}

#[test]
fn string_bodies_with_controls_around_escapes_are_unchanged() {
    // Controls inside `''`, `\\` and `\S\` pairs are skipped by the escape
    // matchers; the literal must end at the real closing quote only.
    let lexed = tokens(b"('a'\n'b',  '\\\n\\', '\\S\\\r'x', 'plain')");
    let texts: Vec<Vec<u8>> = lexed
        .iter()
        .filter_map(|(token, _)| match token {
            Token::Text(value) => Some(value.to_vec()),
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        [
            b"a''b".to_vec(),
            b"\\\\".to_vec(),
            b"\\S\\'x".to_vec(),
            b"plain".to_vec()
        ]
    );
}

#[test]
fn an_unterminated_string_error_spans_to_the_end_of_input() {
    for input in [&b"'no end"[..], b"'ends with backslash\\", b"'x\\S\\"] {
        let error = Lexer::new(input)
            .collect::<Result<Vec<_>, _>>()
            .expect_err("unterminated");
        assert_eq!(error.span(), Span::new(0, input.len()), "{input:?}");
    }
}
