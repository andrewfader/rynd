pub mod ast;
pub mod captures;
pub mod lexer;
pub mod parser;
pub mod token;

pub use ast::{Expr, ExprKind, Literal, Pattern, Program, Stmt};

// Use the language lexer/parser so delimiters in strings and comments don't
// affect continuation, and a trailing operator can continue on the next line.
pub fn needs_more(source: &str) -> bool {
    let tokens = match lexer::Lexer::new(source).tokenize() {
        Ok(tokens) => tokens,
        Err(crate::error::RyndError::LexError { message, .. }) => {
            return message.starts_with("Unterminated string");
        }
        Err(_) => return false,
    };
    let eof = tokens.last().unwrap().span.clone();
    matches!(parser::Parser::new(tokens).parse(), Err(crate::error::RyndError::ParseError { span, .. }) if span == eof)
}
