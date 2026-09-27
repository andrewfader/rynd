use rynd::syntax::lexer::Lexer;
use rynd::syntax::token::TokenType;

#[test]
fn test_lex_basic_literals() {
    let source = "42 3.125 \"hello\\nworld\" true false nil";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should tokenize successfully");

    assert_eq!(tokens[0].token_type, TokenType::Integer(42));
    assert_eq!(tokens[1].token_type, TokenType::Float(3.125));
    assert_eq!(
        tokens[2].token_type,
        TokenType::StringLit("hello\nworld".to_string())
    );
    assert_eq!(tokens[3].token_type, TokenType::True);
    assert_eq!(tokens[4].token_type, TokenType::False);
    assert_eq!(tokens[5].token_type, TokenType::Nil);
}

#[test]
fn test_lex_operators_and_pipeline() {
    let source = "[1, 2, 3] |> filter(\\x -> x > 1) |> sum()";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should tokenize successfully");

    let types: Vec<_> = tokens.into_iter().map(|t| t.token_type).collect();
    assert!(types.contains(&TokenType::PipeRight));
    assert!(types.contains(&TokenType::Arrow));
    assert!(types.contains(&TokenType::Backslash));
}

#[test]
fn test_lex_range_and_float_distinction() {
    let source = "1..10 1.5 20..30";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should tokenize successfully");

    assert_eq!(tokens[0].token_type, TokenType::Integer(1));
    assert_eq!(tokens[1].token_type, TokenType::DotDot);
    assert_eq!(tokens[2].token_type, TokenType::Integer(10));
    assert_eq!(tokens[3].token_type, TokenType::Float(1.5));
    assert_eq!(tokens[4].token_type, TokenType::Integer(20));
    assert_eq!(tokens[5].token_type, TokenType::DotDot);
    assert_eq!(tokens[6].token_type, TokenType::Integer(30));
}

#[test]
fn test_lex_comments() {
    let source = r#"
        # This is a comment
        let x = 10 // inline comment
        // Another comment line
        x + 2
    "#;
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should tokenize successfully");

    assert_eq!(tokens[0].token_type, TokenType::Let);
    assert_eq!(tokens[1].token_type, TokenType::Identifier("x".to_string()));
    assert_eq!(tokens[2].token_type, TokenType::Equal);
    assert_eq!(tokens[3].token_type, TokenType::Integer(10));
    assert_eq!(tokens[4].token_type, TokenType::Identifier("x".to_string()));
    assert_eq!(tokens[5].token_type, TokenType::Plus);
    assert_eq!(tokens[6].token_type, TokenType::Integer(2));
}

#[test]
fn test_lex_elvis_and_safe_nav() {
    let source = "user?.name ?: \"guest\"";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should tokenize successfully");

    assert_eq!(
        tokens[0].token_type,
        TokenType::Identifier("user".to_string())
    );
    assert_eq!(tokens[1].token_type, TokenType::SafeNav);
    assert_eq!(
        tokens[2].token_type,
        TokenType::Identifier("name".to_string())
    );
    assert_eq!(tokens[3].token_type, TokenType::Elvis);
    assert_eq!(
        tokens[4].token_type,
        TokenType::StringLit("guest".to_string())
    );
}

#[test]
fn test_lex_strings_single_quotes_and_escapes() {
    let source = r#"'' 'single quoted' 'can\'t escape \\' "escapes: \n \t \r \" \\ \# \z""#;
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should tokenize strings");

    assert_eq!(tokens[0].token_type, TokenType::StringLit("".to_string()));
    assert_eq!(
        tokens[1].token_type,
        TokenType::StringLit("single quoted".to_string())
    );
    assert_eq!(
        tokens[2].token_type,
        TokenType::StringLit("can't escape \\".to_string())
    );
    assert_eq!(
        tokens[3].token_type,
        TokenType::StringLit("escapes: \n \t \r \" \\ # \\z".to_string())
    );

    // Unterminated single quote error
    let mut bad_single = Lexer::new("'unterminated");
    assert!(bad_single.tokenize().is_err());

    // Unterminated double quote error
    let mut bad_double = Lexer::new("\"unterminated");
    assert!(bad_double.tokenize().is_err());

    // Unterminated escape error
    let mut bad_escape = Lexer::new("\"bad escape \\");
    assert!(bad_escape.tokenize().is_err());
}

#[test]
fn test_lex_string_interpolation() {
    let source = r#""Total: #{cents} cents""#;
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should tokenize interpolation");
    assert_eq!(tokens.len(), 2); // Template + EOF
    match &tokens[0].token_type {
        TokenType::Template(parts) => {
            assert!(parts.len() >= 5);
        }
        other => panic!("Expected Template token, got {other:?}"),
    }

    // Empty interpolation error
    let mut empty_interp = Lexer::new(r#""Empty: #{}""#);
    let err = empty_interp.tokenize().unwrap_err();
    assert!(err.to_string().contains("Empty string interpolation"));

    // Unterminated interpolation error
    let mut unterm_interp = Lexer::new(r#""Unclosed: #{ 1 + 2"#);
    let err = unterm_interp.tokenize().unwrap_err();
    assert!(err.to_string().contains("Unterminated"));
}

#[test]
fn test_lex_numbers_and_scientific_notation() {
    let source = "0 12345 0.0 3.125 1e5 2.5e-3 1E+4 9223372036854775808";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should tokenize numbers");

    assert_eq!(tokens[0].token_type, TokenType::Integer(0));
    assert_eq!(tokens[1].token_type, TokenType::Integer(12345));
    assert_eq!(tokens[2].token_type, TokenType::Float(0.0));
    assert_eq!(tokens[3].token_type, TokenType::Float(3.125));
    assert_eq!(tokens[4].token_type, TokenType::Float(1e5));
    assert_eq!(tokens[5].token_type, TokenType::Float(2.5e-3));
    assert_eq!(tokens[6].token_type, TokenType::Float(1e4));
    assert_eq!(tokens[7].token_type, TokenType::MinIntMagnitude);

    // Invalid exponent digits
    let mut bad_exponent = Lexer::new("1e");
    assert!(bad_exponent.tokenize().is_err());

    // Invalid integer overflow
    let mut overflow_int = Lexer::new("999999999999999999999999999999");
    assert!(overflow_int.tokenize().is_err());
}

#[test]
fn test_lex_all_operators_and_delimiters() {
    let source =
        "+ - * / % == != < <= > >= && || ! |> -> => ?: ? :: : ..= .. . , ; ( ) [ ] { } \\ | _";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should tokenize operators");

    let types: Vec<_> = tokens.into_iter().map(|t| t.token_type).collect();
    let expected = vec![
        TokenType::Plus,
        TokenType::Minus,
        TokenType::Star,
        TokenType::Slash,
        TokenType::Percent,
        TokenType::EqualEqual,
        TokenType::BangEqual,
        TokenType::Less,
        TokenType::LessEqual,
        TokenType::Greater,
        TokenType::GreaterEqual,
        TokenType::AndAnd,
        TokenType::OrOr,
        TokenType::Bang,
        TokenType::PipeRight,
        TokenType::Arrow,
        TokenType::FatArrow,
        TokenType::Elvis,
        TokenType::Question,
        TokenType::DoubleColon,
        TokenType::Colon,
        TokenType::DotDotEqual,
        TokenType::DotDot,
        TokenType::Dot,
        TokenType::Comma,
        TokenType::Semicolon,
        TokenType::LParen,
        TokenType::RParen,
        TokenType::LBracket,
        TokenType::RBracket,
        TokenType::LBrace,
        TokenType::RBrace,
        TokenType::Backslash,
        TokenType::Pipe,
        TokenType::Underscore,
        TokenType::Eof,
    ];
    assert_eq!(types, expected);

    // Lone & error
    let mut lone_amp = Lexer::new("&");
    let err = lone_amp.tokenize().unwrap_err();
    assert!(err.to_string().contains("Expected '&&'"));
}

#[test]
fn test_lex_all_keywords() {
    let source = "fn pub import as unless and or not let mut if else match for in return true false nil Some None Ok Err";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should tokenize keywords");

    let types: Vec<_> = tokens.into_iter().map(|t| t.token_type).collect();
    let expected = vec![
        TokenType::Fn,
        TokenType::Pub,
        TokenType::Import,
        TokenType::As,
        TokenType::Unless,
        TokenType::AndAnd,
        TokenType::OrOr,
        TokenType::Bang,
        TokenType::Let,
        TokenType::Mut,
        TokenType::If,
        TokenType::Else,
        TokenType::Match,
        TokenType::For,
        TokenType::In,
        TokenType::Return,
        TokenType::True,
        TokenType::False,
        TokenType::Nil,
        TokenType::Some,
        TokenType::None,
        TokenType::Ok,
        TokenType::Err,
        TokenType::Eof,
    ];
    assert_eq!(types, expected);
}

#[test]
fn test_lex_unexpected_character_and_span_tracking() {
    let mut bad_char = Lexer::new("@");
    let err = bad_char.tokenize().unwrap_err();
    assert!(err.to_string().contains("Unexpected character: '@'"));

    let source = "a\n  b\n    c";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Should track spans");
    assert_eq!(tokens[0].span.line, 1);
    assert_eq!(tokens[0].span.column, 1);
    assert_eq!(tokens[1].span.line, 2);
    assert_eq!(tokens[1].span.column, 3);
    assert_eq!(tokens[2].span.line, 3);
    assert_eq!(tokens[2].span.column, 5);
}
