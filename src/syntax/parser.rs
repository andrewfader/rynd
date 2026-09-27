use crate::error::{RyndError, RyndResult};
use crate::syntax::ast::*;
use crate::syntax::token::{Token, TokenType};

#[derive(Debug, PartialEq, PartialOrd, Clone, Copy)]
enum Precedence {
    Lowest = 0,
    Ternary = 1,
    Elvis = 2,      // ?:
    LogicalOr = 3,  // ||
    LogicalAnd = 4, // &&
    Equality = 5,   // ==, !=
    Comparison = 6, // <, <=, >, >=
    Pipeline = 7,   // |>
    Range = 8,      // ..
    Term = 9,       // +, -
    Factor = 10,    // *, /, %
    Unary = 11,     // !, -
    Call = 12,      // (), [], ., ?.
}

impl Precedence {
    fn from_token(token: &TokenType) -> Precedence {
        match token {
            TokenType::Question => Precedence::Ternary,
            TokenType::PipeRight => Precedence::Pipeline,
            TokenType::Elvis => Precedence::Elvis,
            TokenType::OrOr => Precedence::LogicalOr,
            TokenType::AndAnd => Precedence::LogicalAnd,
            TokenType::EqualEqual | TokenType::BangEqual => Precedence::Equality,
            TokenType::Less
            | TokenType::LessEqual
            | TokenType::Greater
            | TokenType::GreaterEqual => Precedence::Comparison,
            TokenType::DotDot | TokenType::DotDotEqual => Precedence::Range,
            TokenType::Plus | TokenType::Minus => Precedence::Term,
            TokenType::Star | TokenType::Slash | TokenType::Percent => Precedence::Factor,
            TokenType::LParen | TokenType::LBracket | TokenType::Dot | TokenType::SafeNav => {
                Precedence::Call
            }
            _ => Precedence::Lowest,
        }
    }
}

pub struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    pub fn new(mut tokens: Vec<Token>) -> Self {
        if !tokens
            .last()
            .is_some_and(|t| t.token_type == TokenType::Eof)
        {
            let span = tokens
                .last()
                .map(|t| t.span.clone())
                .unwrap_or_else(|| crate::error::Span::new(1, 1));
            tokens.push(Token::new(TokenType::Eof, span));
        }
        Self { tokens, cursor: 0 }
    }

    pub fn parse(&mut self) -> RyndResult<Program> {
        let mut statements = Vec::new();
        while !self.is_at_end() {
            // Skip any redundant semicolons
            while self.match_token(&[TokenType::Semicolon]) {}
            if self.is_at_end() {
                break;
            }
            statements.push(self.parse_statement()?);
            while self.match_token(&[TokenType::Semicolon]) {}
        }
        Ok(Program { statements })
    }

    fn parse_statement(&mut self) -> RyndResult<Stmt> {
        if self.match_token(&[TokenType::Pub]) {
            if !self.check(&TokenType::Fn) && !self.check(&TokenType::Let) {
                return Err(RyndError::ParseError {
                    message: "pub must precede fn or let".into(),
                    span: self.peek().span.clone(),
                });
            }
            return Ok(Stmt::Public(Box::new(self.parse_statement()?)));
        }
        if self.check(&TokenType::Import) {
            let span = self.advance().span;
            let token = self.advance();
            let TokenType::StringLit(path) = token.token_type else {
                return Err(RyndError::ParseError {
                    message: "Expected a literal module path".into(),
                    span: token.span,
                });
            };
            self.consume(TokenType::As, "Expected 'as' after module path")?;
            let alias = self.consume_identifier("Expected module alias")?;
            let TokenType::Identifier(alias) = alias.token_type else {
                unreachable!()
            };
            return Ok(Stmt::Import { path, alias, span });
        }
        if self.check(&TokenType::Let) {
            self.parse_let_statement()
        } else if self.check(&TokenType::Fn) {
            self.parse_fn_statement()
        } else if self.check(&TokenType::Return) {
            self.parse_return_statement()
        } else {
            let expr = self.parse_statement_expression()?;
            Ok(Stmt::Expression(expr))
        }
    }

    fn parse_statement_expression(&mut self) -> RyndResult<Expr> {
        let body = self.parse_expression(Precedence::Lowest)?;
        self.apply_statement_modifier(body)
    }

    fn apply_statement_modifier(&mut self, body: Expr) -> RyndResult<Expr> {
        // Keep comprehension filters and match guards in their own grammar.
        if self.peek().span.line == self.tokens[self.cursor - 1].span.line
            && matches!(self.peek().token_type, TokenType::If | TokenType::Unless)
        {
            let modifier = self.advance();
            let mut condition = self.parse_expression(Precedence::Lowest)?;
            if modifier.token_type == TokenType::Unless {
                condition = Expr::new(
                    ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(condition),
                    },
                    modifier.span.clone(),
                );
            }
            Ok(Expr::new(
                ExprKind::If {
                    condition: Box::new(condition),
                    then_branch: Box::new(body),
                    else_branch: None,
                },
                modifier.span,
            ))
        } else {
            Ok(body)
        }
    }

    fn parse_let_statement(&mut self) -> RyndResult<Stmt> {
        let let_token = self.consume(TokenType::Let, "Expected 'let'")?;
        let is_mut = self.match_token(&[TokenType::Mut]);
        let pattern = self.parse_pattern()?;
        self.consume(TokenType::Equal, "Expected '=' in variable declaration")?;
        let init = self.parse_expression(Precedence::Lowest)?;
        Ok(Stmt::Let {
            pattern,
            init,
            is_mut,
            span: let_token.span,
        })
    }

    fn parse_fn_statement(&mut self) -> RyndResult<Stmt> {
        let fn_token = self.consume(TokenType::Fn, "Expected 'fn'")?;
        let name_token = self.consume_identifier("Expected function name")?;
        let name = match name_token.token_type {
            TokenType::Identifier(s) => s,
            _ => unreachable!(),
        };

        self.consume(TokenType::LParen, "Expected '(' after function name")?;
        let (params, bindings) = self.parse_parameters(&TokenType::RParen)?;
        self.consume(TokenType::RParen, "Expected ')' after parameters")?;

        let body = self.parse_block_expression()?;
        let body = Self::bind_parameters(body, bindings);
        Ok(Stmt::Function {
            name,
            params,
            body,
            span: fn_token.span,
        })
    }

    fn parse_return_statement(&mut self) -> RyndResult<Stmt> {
        let ret_token = self.consume(TokenType::Return, "Expected 'return'")?;
        let value = if self.check(&TokenType::Semicolon)
            || self.check(&TokenType::RBrace)
            || self.is_at_end()
        {
            None
        } else {
            Some(self.parse_expression(Precedence::Lowest)?)
        };
        let span = ret_token.span;
        let statement = Stmt::Return {
            value,
            span: span.clone(),
        };
        if self.peek().span.line == self.tokens[self.cursor - 1].span.line
            && matches!(self.peek().token_type, TokenType::If | TokenType::Unless)
        {
            let body = Expr::new(
                ExprKind::Block {
                    statements: vec![statement],
                    final_expr: None,
                },
                span,
            );
            Ok(Stmt::Expression(self.apply_statement_modifier(body)?))
        } else {
            Ok(statement)
        }
    }

    fn parse_pattern(&mut self) -> RyndResult<Pattern> {
        if self.match_token(&[TokenType::Minus]) {
            let token = self.advance();
            return match token.token_type {
                TokenType::MinIntMagnitude => Ok(Pattern::Literal(Literal::Int(i64::MIN))),
                TokenType::Integer(i) => Ok(Pattern::Literal(Literal::Int(-i))),
                TokenType::Float(f) => Ok(Pattern::Literal(Literal::Float(-f))),
                _ => Err(RyndError::ParseError {
                    message: "Expected number after '-' in pattern".into(),
                    span: token.span,
                }),
            };
        }
        if self.match_token(&[TokenType::Underscore]) {
            Ok(Pattern::Wildcard)
        } else if self.check(&TokenType::LParen) {
            self.advance();
            let mut sub_patterns = Vec::new();
            if !self.check(&TokenType::RParen) {
                loop {
                    sub_patterns.push(self.parse_pattern()?);
                    if !self.match_token(&[TokenType::Comma]) || self.check(&TokenType::RParen) {
                        break;
                    }
                }
            }
            self.consume(TokenType::RParen, "Expected ')' in tuple pattern")?;
            Ok(Pattern::Tuple(sub_patterns))
        } else if self.check(&TokenType::LBracket) {
            self.advance();
            let mut sub_patterns = Vec::new();
            if !self.check(&TokenType::RBracket) {
                loop {
                    sub_patterns.push(self.parse_pattern()?);
                    if !self.match_token(&[TokenType::Comma]) || self.check(&TokenType::RBracket) {
                        break;
                    }
                }
            }
            self.consume(TokenType::RBracket, "Expected ']' in list pattern")?;
            Ok(Pattern::List(sub_patterns))
        } else if self.match_token(&[TokenType::Some]) {
            self.consume(TokenType::LParen, "Expected '(' after Some")?;
            let inner = self.parse_pattern()?;
            self.match_token(&[TokenType::Comma]);
            self.consume(TokenType::RParen, "Expected ')' after Some pattern")?;
            Ok(Pattern::Variant {
                name: "Some".to_string(),
                args: vec![inner],
            })
        } else if self.match_token(&[TokenType::None]) {
            Ok(Pattern::Variant {
                name: "None".to_string(),
                args: vec![],
            })
        } else if self.match_token(&[TokenType::Ok]) {
            self.consume(TokenType::LParen, "Expected '(' after Ok")?;
            let inner = self.parse_pattern()?;
            self.match_token(&[TokenType::Comma]);
            self.consume(TokenType::RParen, "Expected ')' after Ok pattern")?;
            Ok(Pattern::Variant {
                name: "Ok".to_string(),
                args: vec![inner],
            })
        } else if self.match_token(&[TokenType::Err]) {
            self.consume(TokenType::LParen, "Expected '(' after Err")?;
            let inner = self.parse_pattern()?;
            self.match_token(&[TokenType::Comma]);
            self.consume(TokenType::RParen, "Expected ')' after Err pattern")?;
            Ok(Pattern::Variant {
                name: "Err".to_string(),
                args: vec![inner],
            })
        } else {
            let tok = self.advance();
            match tok.token_type {
                TokenType::Integer(i) => Ok(Pattern::Literal(Literal::Int(i))),
                TokenType::Float(f) => Ok(Pattern::Literal(Literal::Float(f))),
                TokenType::StringLit(s) => Ok(Pattern::Literal(Literal::String(s))),
                TokenType::True => Ok(Pattern::Literal(Literal::Bool(true))),
                TokenType::False => Ok(Pattern::Literal(Literal::Bool(false))),
                TokenType::Nil => Ok(Pattern::Literal(Literal::Nil)),
                TokenType::Identifier(id) => {
                    // Check if followed by ( for variant pattern e.g. Color(r, g, b)
                    if self.check(&TokenType::LParen) {
                        self.advance();
                        let mut args = Vec::new();
                        if !self.check(&TokenType::RParen) {
                            loop {
                                args.push(self.parse_pattern()?);
                                if !self.match_token(&[TokenType::Comma])
                                    || self.check(&TokenType::RParen)
                                {
                                    break;
                                }
                            }
                        }
                        self.consume(TokenType::RParen, "Expected ')' in variant pattern")?;
                        Ok(Pattern::Variant { name: id, args })
                    } else {
                        Ok(Pattern::Variable(id))
                    }
                }
                _ => Err(RyndError::ParseError {
                    message: format!("Expected pattern, found {:?}", tok.token_type),
                    span: tok.span,
                }),
            }
        }
    }

    fn parse_expression(&mut self, precedence: Precedence) -> RyndResult<Expr> {
        let mut left = self.parse_prefix()?;

        while !self.is_at_end() && precedence < Precedence::from_token(&self.peek().token_type) {
            // Avoid parsing '[' or '(' across newlines as call/index of previous expression
            if self.peek().span.line > self.tokens[self.cursor - 1].span.line
                && matches!(
                    self.peek().token_type,
                    TokenType::LParen | TokenType::LBracket
                )
            {
                break;
            }
            left = self.parse_infix(left)?;
        }

        Ok(left)
    }

    fn parse_prefix(&mut self) -> RyndResult<Expr> {
        let token = self.peek().clone();
        let span = token.span.clone();

        match token.token_type {
            TokenType::Integer(i) => {
                self.advance();
                Ok(Expr::new(ExprKind::Literal(Literal::Int(i)), span))
            }
            TokenType::Float(f) => {
                self.advance();
                Ok(Expr::new(ExprKind::Literal(Literal::Float(f)), span))
            }
            TokenType::Template(tokens) => {
                self.advance();
                let mut parser = Parser::new(tokens);
                let expression = parser.parse_expression(Precedence::Lowest)?;
                if !parser.is_at_end() {
                    return Err(RyndError::ParseError {
                        message: "Expected one expression in interpolation".into(),
                        span: parser.peek().span.clone(),
                    });
                }
                Ok(expression)
            }
            TokenType::StringLit(s) => {
                self.advance();
                Ok(Expr::new(ExprKind::Literal(Literal::String(s)), span))
            }
            TokenType::True => {
                self.advance();
                Ok(Expr::new(ExprKind::Literal(Literal::Bool(true)), span))
            }
            TokenType::False => {
                self.advance();
                Ok(Expr::new(ExprKind::Literal(Literal::Bool(false)), span))
            }
            TokenType::Nil => {
                self.advance();
                Ok(Expr::new(ExprKind::Literal(Literal::Nil), span))
            }
            TokenType::Underscore => {
                self.advance();
                Ok(Expr::new(ExprKind::Identifier("_".into()), span))
            }
            TokenType::Identifier(name) => {
                self.advance();
                Ok(Expr::new(ExprKind::Identifier(name), span))
            }
            TokenType::Minus => {
                self.advance();
                if self.match_token(&[TokenType::MinIntMagnitude]) {
                    return Ok(Expr::new(ExprKind::Literal(Literal::Int(i64::MIN)), span));
                }
                let operand = self.parse_expression(Precedence::Unary)?;
                Ok(Expr::new(
                    ExprKind::Unary {
                        op: UnaryOp::Neg,
                        operand: Box::new(operand),
                    },
                    span,
                ))
            }
            TokenType::Bang => {
                self.advance();
                let operand = self.parse_expression(Precedence::Unary)?;
                Ok(Expr::new(
                    ExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(operand),
                    },
                    span,
                ))
            }
            TokenType::LParen => {
                self.advance();
                if self.match_token(&[TokenType::RParen]) {
                    // Empty tuple
                    return Ok(Expr::new(ExprKind::Tuple(Vec::new()), span));
                }
                let first = self.parse_expression(Precedence::Lowest)?;
                if self.match_token(&[TokenType::Comma]) {
                    // It's a tuple!
                    let mut elements = vec![first];
                    if !self.check(&TokenType::RParen) {
                        loop {
                            elements.push(self.parse_expression(Precedence::Lowest)?);
                            if !self.match_token(&[TokenType::Comma])
                                || self.check(&TokenType::RParen)
                            {
                                break;
                            }
                        }
                    }
                    self.consume(TokenType::RParen, "Expected ')' after tuple elements")?;
                    Ok(Expr::new(ExprKind::Tuple(elements), span))
                } else {
                    self.consume(TokenType::RParen, "Expected ')' after expression")?;
                    Ok(first)
                }
            }
            TokenType::LBracket => self.parse_bracket_expression(),
            TokenType::LBrace => self.parse_brace_expression(),
            TokenType::Backslash | TokenType::Pipe => self.parse_lambda_expression(),
            TokenType::If | TokenType::Unless => self.parse_if_expression(),
            TokenType::Match => self.parse_match_expression(),
            TokenType::Some => {
                self.advance();
                self.consume(TokenType::LParen, "Expected '(' after Some")?;
                let inner = self.parse_expression(Precedence::Lowest)?;
                self.match_token(&[TokenType::Comma]);
                self.consume(TokenType::RParen, "Expected ')' after Some expression")?;
                Ok(Expr::new(
                    ExprKind::Call {
                        callee: Box::new(Expr::new(
                            ExprKind::Identifier("Some".to_string()),
                            span.clone(),
                        )),
                        args: vec![inner],
                    },
                    span,
                ))
            }
            TokenType::None => {
                self.advance();
                Ok(Expr::new(ExprKind::Identifier("None".to_string()), span))
            }
            TokenType::Ok => {
                self.advance();
                self.consume(TokenType::LParen, "Expected '(' after Ok")?;
                let inner = self.parse_expression(Precedence::Lowest)?;
                self.match_token(&[TokenType::Comma]);
                self.consume(TokenType::RParen, "Expected ')' after Ok expression")?;
                Ok(Expr::new(
                    ExprKind::Call {
                        callee: Box::new(Expr::new(
                            ExprKind::Identifier("Ok".to_string()),
                            span.clone(),
                        )),
                        args: vec![inner],
                    },
                    span,
                ))
            }
            TokenType::Err => {
                self.advance();
                self.consume(TokenType::LParen, "Expected '(' after Err")?;
                let inner = self.parse_expression(Precedence::Lowest)?;
                self.match_token(&[TokenType::Comma]);
                self.consume(TokenType::RParen, "Expected ')' after Err expression")?;
                Ok(Expr::new(
                    ExprKind::Call {
                        callee: Box::new(Expr::new(
                            ExprKind::Identifier("Err".to_string()),
                            span.clone(),
                        )),
                        args: vec![inner],
                    },
                    span,
                ))
            }
            _ => Err(RyndError::ParseError {
                message: format!("Unexpected token in expression: {:?}", token.token_type),
                span,
            }),
        }
    }

    fn parse_bracket_expression(&mut self) -> RyndResult<Expr> {
        let start_tok = self.consume(TokenType::LBracket, "Expected '['")?;
        let span = start_tok.span;

        if self.match_token(&[TokenType::RBracket]) {
            return Ok(Expr::new(ExprKind::List(Vec::new()), span));
        }

        let first = self.parse_expression(Precedence::Lowest)?;

        // Check if this is a list comprehension: [ expr for var in iter (if cond)? ]
        if self.match_token(&[TokenType::For]) {
            let var_tok = self.consume_identifier("Expected variable name after 'for'")?;
            let var_name = match var_tok.token_type {
                TokenType::Identifier(id) => id,
                _ => unreachable!(),
            };
            self.consume(TokenType::In, "Expected 'in' after comprehension variable")?;
            let iterable = self.parse_expression(Precedence::Lowest)?;

            let condition = if self.match_token(&[TokenType::If]) {
                Some(Box::new(self.parse_expression(Precedence::Lowest)?))
            } else {
                None
            };

            self.consume(TokenType::RBracket, "Expected ']' at end of comprehension")?;
            return Ok(Expr::new(
                ExprKind::Comprehension {
                    element: Box::new(first),
                    variable: var_name,
                    iterable: Box::new(iterable),
                    condition,
                },
                span,
            ));
        }

        // Otherwise it's a standard list literal: [elem1, elem2, ...]
        let mut elements = vec![first];
        while self.match_token(&[TokenType::Comma]) {
            if self.check(&TokenType::RBracket) {
                break;
            }
            elements.push(self.parse_expression(Precedence::Lowest)?);
        }
        self.consume(TokenType::RBracket, "Expected ']' at end of list")?;
        Ok(Expr::new(ExprKind::List(elements), span))
    }

    fn parse_brace_expression(&mut self) -> RyndResult<Expr> {
        if self.tokens.get(self.cursor + 1).is_some_and(|t| {
            matches!(
                t.token_type,
                TokenType::Let | TokenType::Fn | TokenType::Return
            )
        }) {
            return self.parse_block_expression();
        }
        let start_tok = self.consume(TokenType::LBrace, "Expected '{'")?;
        let span = start_tok.span;

        if self.match_token(&[TokenType::RBrace]) {
            return Ok(Expr::new(ExprKind::Map(Vec::new()), span));
        }

        // Lookahead to distinguish between Map literal { k: v } and Block { stmts }
        // If the next tokens match `expr : expr`, it's a Map!
        // We can parse an expression first, then check if ':' follows
        let first_expr = self.parse_statement_expression()?;
        if self.match_token(&[TokenType::Colon]) {
            let first_val = self.parse_expression(Precedence::Lowest)?;
            if self.match_token(&[TokenType::For]) {
                let variable = match self
                    .consume_identifier("Expected comprehension variable")?
                    .token_type
                {
                    TokenType::Identifier(name) => name,
                    _ => unreachable!(),
                };
                self.consume(TokenType::In, "Expected 'in'")?;
                let iterable = Box::new(self.parse_expression(Precedence::Lowest)?);
                let condition = if self.match_token(&[TokenType::If]) {
                    Some(Box::new(self.parse_expression(Precedence::Lowest)?))
                } else {
                    None
                };
                self.consume(TokenType::RBrace, "Expected '}' after map comprehension")?;
                return Ok(Expr::new(
                    ExprKind::MapComprehension {
                        key: Box::new(first_expr),
                        value: Box::new(first_val),
                        variable,
                        iterable,
                        condition,
                    },
                    span,
                ));
            }
            let mut pairs = vec![(first_expr, first_val)];
            while self.match_token(&[TokenType::Comma]) {
                if self.check(&TokenType::RBrace) {
                    break;
                }
                let k = self.parse_expression(Precedence::Lowest)?;
                self.consume(TokenType::Colon, "Expected ':' in key-value pair")?;
                let v = self.parse_expression(Precedence::Lowest)?;
                pairs.push((k, v));
            }
            self.consume(TokenType::RBrace, "Expected '}' after map entries")?;
            return Ok(Expr::new(ExprKind::Map(pairs), span));
        }

        // It is a Block! The first_expr is either a statement (if followed by semicolon/statement) or final expression
        let mut statements = Vec::new();
        let mut final_expr = None;

        if self.match_token(&[TokenType::Semicolon]) {
            statements.push(Stmt::Expression(first_expr));
            self.parse_remaining_block_statements(&mut statements, &mut final_expr)?;
        } else if self.check(&TokenType::RBrace) {
            self.advance();
            return Ok(Expr::new(
                ExprKind::Block {
                    statements,
                    final_expr: Some(Box::new(first_expr)),
                },
                span,
            ));
        } else {
            // Could be another statement without semicolon on newline
            statements.push(Stmt::Expression(first_expr));
            self.parse_remaining_block_statements(&mut statements, &mut final_expr)?;
        }

        Ok(Expr::new(
            ExprKind::Block {
                statements,
                final_expr,
            },
            span,
        ))
    }

    fn parse_remaining_block_statements(
        &mut self,
        statements: &mut Vec<Stmt>,
        final_expr: &mut Option<Box<Expr>>,
    ) -> RyndResult<()> {
        while !self.check(&TokenType::RBrace) && !self.is_at_end() {
            while self.match_token(&[TokenType::Semicolon]) {}
            if self.check(&TokenType::RBrace) || self.is_at_end() {
                break;
            }

            if self.check(&TokenType::Pub)
                || self.check(&TokenType::Import)
                || self.check(&TokenType::Let)
                || self.check(&TokenType::Fn)
                || self.check(&TokenType::Return)
            {
                statements.push(self.parse_statement()?);
            } else {
                let expr = self.parse_statement_expression()?;
                if self.match_token(&[TokenType::Semicolon]) {
                    statements.push(Stmt::Expression(expr));
                } else if self.check(&TokenType::RBrace) {
                    *final_expr = Some(Box::new(expr));
                    break;
                } else {
                    statements.push(Stmt::Expression(expr));
                }
            }
        }
        self.consume(TokenType::RBrace, "Expected '}' at end of block")?;
        Ok(())
    }

    fn parse_block_expression(&mut self) -> RyndResult<Expr> {
        let start_tok = self.consume(TokenType::LBrace, "Expected '{' to start block")?;
        let span = start_tok.span;
        let mut statements = Vec::new();
        let mut final_expr = None;

        while !self.check(&TokenType::RBrace) && !self.is_at_end() {
            while self.match_token(&[TokenType::Semicolon]) {}
            if self.check(&TokenType::RBrace) || self.is_at_end() {
                break;
            }

            if self.check(&TokenType::Pub)
                || self.check(&TokenType::Import)
                || self.check(&TokenType::Let)
                || self.check(&TokenType::Fn)
                || self.check(&TokenType::Return)
            {
                statements.push(self.parse_statement()?);
            } else {
                let expr = self.parse_statement_expression()?;
                if self.match_token(&[TokenType::Semicolon]) {
                    statements.push(Stmt::Expression(expr));
                } else if self.check(&TokenType::RBrace) {
                    final_expr = Some(Box::new(expr));
                    break;
                } else {
                    statements.push(Stmt::Expression(expr));
                }
            }
        }
        self.consume(TokenType::RBrace, "Expected '}' at end of block")?;

        Ok(Expr::new(
            ExprKind::Block {
                statements,
                final_expr,
            },
            span,
        ))
    }

    fn parse_parameters(&mut self, end: &TokenType) -> RyndResult<(Vec<String>, Vec<Stmt>)> {
        let mut params = Vec::new();
        let mut bindings = Vec::new();
        let mut names = std::collections::HashSet::new();
        while !self.check(end) && !self.is_at_end() {
            let span = self.peek().span.clone();
            let pattern = self.parse_pattern()?;
            for name in crate::vm::runtime::pattern_names(&pattern) {
                if !names.insert(name) {
                    return Err(RyndError::ParseError {
                        message: "Duplicate function parameter".into(),
                        span,
                    });
                }
            }
            if let Pattern::Variable(name) = pattern {
                params.push(name);
            } else {
                let name = format!("@argument{}", params.len());
                params.push(name.clone());
                bindings.push(Stmt::Let {
                    pattern,
                    init: Expr::new(ExprKind::Identifier(name), span.clone()),
                    is_mut: false,
                    span,
                });
            }
            if !self.match_token(&[TokenType::Comma]) {
                break;
            }
        }
        Ok((params, bindings))
    }
    fn bind_parameters(body: Expr, bindings: Vec<Stmt>) -> Expr {
        if bindings.is_empty() {
            body
        } else {
            let span = body.span.clone();
            Expr::new(
                ExprKind::Block {
                    statements: bindings,
                    final_expr: Some(Box::new(body)),
                },
                span,
            )
        }
    }

    fn parse_lambda_expression(&mut self) -> RyndResult<Expr> {
        let start_tok = self.advance(); // consume '\' or '|'
        let span = start_tok.span;
        let is_rust_style = start_tok.token_type == TokenType::Pipe;

        let end_token = if is_rust_style {
            TokenType::Pipe
        } else {
            TokenType::Arrow
        };
        let (params, bindings) = self.parse_parameters(&end_token)?;

        if is_rust_style {
            self.consume(
                TokenType::Pipe,
                "Expected closing '|' in lambda parameter list",
            )?;
        } else {
            self.consume(TokenType::Arrow, "Expected '->' after lambda parameters")?;
        }

        let body = self.parse_expression(Precedence::Lowest)?;
        let body = Self::bind_parameters(body, bindings);
        Ok(Expr::new(
            ExprKind::Lambda {
                params,
                body: Box::new(body),
            },
            span,
        ))
    }

    fn parse_if_expression(&mut self) -> RyndResult<Expr> {
        let if_tok = self.advance();
        let unless = if_tok.token_type == TokenType::Unless;
        let span = if_tok.span;
        let mut condition = self.parse_expression(Precedence::Lowest)?;
        if unless {
            condition = Expr::new(
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(condition),
                },
                span.clone(),
            );
        }
        let then_branch = self.parse_block_expression()?;

        let else_branch = if self.match_token(&[TokenType::Else]) {
            if self.check(&TokenType::If) || self.check(&TokenType::Unless) {
                Some(Box::new(self.parse_if_expression()?))
            } else {
                Some(Box::new(self.parse_block_expression()?))
            }
        } else {
            None
        };

        Ok(Expr::new(
            ExprKind::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch,
            },
            span,
        ))
    }

    fn parse_match_expression(&mut self) -> RyndResult<Expr> {
        let match_tok = self.consume(TokenType::Match, "Expected 'match'")?;
        let span = match_tok.span;
        let target = self.parse_expression(Precedence::Lowest)?;

        self.consume(TokenType::LBrace, "Expected '{' after match expression")?;
        let mut arms = Vec::new();

        while !self.check(&TokenType::RBrace) && !self.is_at_end() {
            let pattern = self.parse_pattern()?;
            let guard = if self.match_token(&[TokenType::If]) {
                Some(self.parse_expression(Precedence::Lowest)?)
            } else {
                None
            };
            self.consume(TokenType::FatArrow, "Expected '=>' after match pattern")?;

            let body = if self.check(&TokenType::LBrace) {
                self.parse_block_expression()?
            } else {
                self.parse_expression(Precedence::Lowest)?
            };

            arms.push(MatchArm {
                pattern,
                guard,
                body,
            });

            self.match_token(&[TokenType::Comma]);
        }

        self.consume(TokenType::RBrace, "Expected '}' at end of match")?;

        Ok(Expr::new(
            ExprKind::Match {
                target: Box::new(target),
                arms,
            },
            span,
        ))
    }

    fn parse_infix(&mut self, left: Expr) -> RyndResult<Expr> {
        let token = self.peek().clone();
        let span = token.span.clone();

        match token.token_type {
            TokenType::Question => {
                self.advance();
                let then_branch = self.parse_expression(Precedence::Lowest)?;
                self.consume(TokenType::Colon, "Expected ':' in ternary expression")?;
                let else_branch = self.parse_expression(Precedence::Lowest)?;
                Ok(Expr::new(
                    ExprKind::If {
                        condition: Box::new(left),
                        then_branch: Box::new(then_branch),
                        else_branch: Some(Box::new(else_branch)),
                    },
                    span,
                ))
            }
            TokenType::PipeRight => {
                self.advance();
                let right = self.parse_expression(Precedence::Pipeline)?;
                Ok(Expr::new(
                    ExprKind::Pipeline {
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ))
            }
            TokenType::Elvis => {
                self.advance();
                let right = self.parse_expression(Precedence::Elvis)?;
                Ok(Expr::new(
                    ExprKind::Elvis {
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ))
            }
            TokenType::DotDot | TokenType::DotDotEqual => {
                let inclusive = token.token_type == TokenType::DotDotEqual;
                self.advance();
                let right = self.parse_expression(Precedence::Range)?;
                Ok(Expr::new(
                    ExprKind::Range {
                        start: Box::new(left),
                        end: Box::new(right),
                        inclusive,
                    },
                    span,
                ))
            }
            TokenType::LParen => {
                self.advance();
                let mut args = Vec::new();
                if !self.check(&TokenType::RParen) {
                    loop {
                        args.push(self.parse_expression(Precedence::Lowest)?);
                        if !self.match_token(&[TokenType::Comma]) || self.check(&TokenType::RParen)
                        {
                            break;
                        }
                    }
                }
                self.consume(TokenType::RParen, "Expected ')' after call arguments")?;
                Ok(Expr::new(
                    ExprKind::Call {
                        callee: Box::new(left),
                        args,
                    },
                    span,
                ))
            }
            TokenType::LBracket => {
                self.advance();
                let index = self.parse_expression(Precedence::Lowest)?;
                self.consume(TokenType::RBracket, "Expected ']' after index")?;
                Ok(Expr::new(
                    ExprKind::Index {
                        target: Box::new(left),
                        index: Box::new(index),
                    },
                    span,
                ))
            }
            TokenType::Dot => {
                self.advance();
                let field_tok = self.consume_identifier("Expected field name after '.'")?;
                let field = match field_tok.token_type {
                    TokenType::Identifier(id) => id,
                    _ => unreachable!(),
                };
                Ok(Expr::new(
                    ExprKind::FieldAccess {
                        target: Box::new(left),
                        field,
                    },
                    span,
                ))
            }
            TokenType::SafeNav => {
                self.advance();
                let field_tok = self.consume_identifier("Expected field name after '?.'")?;
                let field = match field_tok.token_type {
                    TokenType::Identifier(id) => id,
                    _ => unreachable!(),
                };
                Ok(Expr::new(
                    ExprKind::SafeFieldAccess {
                        target: Box::new(left),
                        field,
                    },
                    span,
                ))
            }
            TokenType::Plus
            | TokenType::Minus
            | TokenType::Star
            | TokenType::Slash
            | TokenType::Percent
            | TokenType::EqualEqual
            | TokenType::BangEqual
            | TokenType::Less
            | TokenType::LessEqual
            | TokenType::Greater
            | TokenType::GreaterEqual
            | TokenType::AndAnd
            | TokenType::OrOr => {
                let op = match token.token_type {
                    TokenType::Plus => BinaryOp::Add,
                    TokenType::Minus => BinaryOp::Sub,
                    TokenType::Star => BinaryOp::Mul,
                    TokenType::Slash => BinaryOp::Div,
                    TokenType::Percent => BinaryOp::Mod,
                    TokenType::EqualEqual => BinaryOp::Equal,
                    TokenType::BangEqual => BinaryOp::NotEqual,
                    TokenType::Less => BinaryOp::Less,
                    TokenType::LessEqual => BinaryOp::LessEqual,
                    TokenType::Greater => BinaryOp::Greater,
                    TokenType::GreaterEqual => BinaryOp::GreaterEqual,
                    TokenType::AndAnd => BinaryOp::And,
                    TokenType::OrOr => BinaryOp::Or,
                    _ => unreachable!(),
                };
                let prec = Precedence::from_token(&token.token_type);
                self.advance();
                let right = self.parse_expression(prec)?;
                Ok(Expr::new(
                    ExprKind::Binary {
                        op,
                        left: Box::new(left),
                        right: Box::new(right),
                    },
                    span,
                ))
            }
            _ => Err(RyndError::ParseError {
                message: format!("Unexpected infix token: {:?}", token.token_type),
                span,
            }),
        }
    }

    fn check(&self, token_type: &TokenType) -> bool {
        if self.is_at_end() {
            false
        } else {
            &self.peek().token_type == token_type
        }
    }

    fn match_token(&mut self, types: &[TokenType]) -> bool {
        for t in types {
            if self.check(t) {
                self.advance();
                return true;
            }
        }
        false
    }

    fn consume(&mut self, token_type: TokenType, err_msg: &str) -> RyndResult<Token> {
        if self.check(&token_type) {
            Ok(self.advance())
        } else {
            let tok = self.peek();
            Err(RyndError::ParseError {
                message: format!("{err_msg}, found {:?}", tok.token_type),
                span: tok.span.clone(),
            })
        }
    }

    fn consume_identifier(&mut self, err_msg: &str) -> RyndResult<Token> {
        if let TokenType::Identifier(_) = self.peek().token_type {
            Ok(self.advance())
        } else {
            let tok = self.peek();
            Err(RyndError::ParseError {
                message: format!("{err_msg}, found {:?}", tok.token_type),
                span: tok.span.clone(),
            })
        }
    }

    fn peek(&self) -> &Token {
        if self.cursor >= self.tokens.len() {
            self.tokens.last().unwrap()
        } else {
            &self.tokens[self.cursor]
        }
    }

    fn advance(&mut self) -> Token {
        let token = self.peek().clone();
        if !self.is_at_end() {
            self.cursor += 1;
        }
        token
    }

    fn is_at_end(&self) -> bool {
        self.peek().token_type == TokenType::Eof
    }
}
