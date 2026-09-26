use crate::error::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    // Literals
    Integer(i64),
    MinIntMagnitude,
    Float(f64),
    StringLit(String),
    Template(Vec<Token>),
    Identifier(String),

    // Keywords
    Fn,
    Pub,
    Import,
    As,
    Unless,
    Let,
    Mut,
    If,
    Else,
    Match,
    For,
    In,
    Return,
    True,
    False,
    Nil,
    Some,
    None,
    Ok,
    Err,

    // Operators
    Plus,         // +
    Minus,        // -
    Star,         // *
    Slash,        // /
    Percent,      // %
    EqualEqual,   // ==
    BangEqual,    // !=
    Less,         // <
    LessEqual,    // <=
    Greater,      // >
    GreaterEqual, // >=
    AndAnd,       // &&
    OrOr,         // ||
    Bang,         // !
    PipeRight,    // |>
    Arrow,        // ->
    FatArrow,     // =>
    Elvis,        // ?:
    Question,     // condition ? yes : no
    SafeNav,      // ?.
    Dot,          // .
    DotDotEqual,
    DotDot,    // ..
    Equal,     // =
    Pipe,      // |
    Backslash, // \

    // Delimiters
    Comma,       // ,
    Colon,       // :
    DoubleColon, // ::
    Semicolon,   // ;
    LParen,      // (
    RParen,      // )
    LBracket,    // [
    RBracket,    // ]
    LBrace,      // {
    RBrace,      // }
    Underscore,  // _

    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub token_type: TokenType,
    pub span: Span,
}

impl Token {
    pub fn new(token_type: TokenType, span: Span) -> Self {
        Self { token_type, span }
    }
}
