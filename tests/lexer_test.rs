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
