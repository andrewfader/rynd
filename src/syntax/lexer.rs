use crate::error::{RyndError, RyndResult, Span};
use crate::syntax::token::{Token, TokenType};

pub struct Lexer<'a> {
    source: &'a str,
    cursor: usize,
    line: usize,
    column: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            cursor: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn tokenize(&mut self) -> RyndResult<Vec<Token>> {
        self.tokenize_inner(false)
    }

    fn tokenize_inner(&mut self, interpolation: bool) -> RyndResult<Vec<Token>> {
        let mut tokens = Vec::new();
        let mut braces = 0usize;
        while !self.is_at_end() {
            self.skip_whitespace_and_comments();
            if self.is_at_end() {
                break;
            }

            let span = Span::new(self.line, self.column);
            let ch = self.peek();
            if interpolation && ch == '}' && braces == 0 {
                self.advance();
                tokens.push(Token::new(TokenType::Eof, span));
                return Ok(tokens);
            }

            match ch {
                '(' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::LParen, span));
                }
                ')' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::RParen, span));
                }
                '[' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::LBracket, span));
                }
                ']' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::RBracket, span));
                }
                '{' => {
                    braces += 1;
                    self.advance();
                    tokens.push(Token::new(TokenType::LBrace, span));
                }
                '}' => {
                    braces = braces.saturating_sub(1);
                    self.advance();
                    tokens.push(Token::new(TokenType::RBrace, span));
                }
                ',' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::Comma, span));
                }
                ';' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::Semicolon, span));
                }
                '\\' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::Backslash, span));
                }
                '+' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::Plus, span));
                }
                '-' => {
                    self.advance();
                    if self.peek() == '>' {
                        self.advance();
                        tokens.push(Token::new(TokenType::Arrow, span));
                    } else {
                        tokens.push(Token::new(TokenType::Minus, span));
                    }
                }
                '*' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::Star, span));
                }
                '/' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::Slash, span));
                }
                '%' => {
                    self.advance();
                    tokens.push(Token::new(TokenType::Percent, span));
                }
                '|' => {
                    self.advance();
                    if self.peek() == '>' {
                        self.advance();
                        tokens.push(Token::new(TokenType::PipeRight, span));
                    } else if self.peek() == '|' {
                        self.advance();
                        tokens.push(Token::new(TokenType::OrOr, span));
                    } else {
                        tokens.push(Token::new(TokenType::Pipe, span));
                    }
                }
                '&' => {
                    self.advance();
                    if self.peek() == '&' {
                        self.advance();
                        tokens.push(Token::new(TokenType::AndAnd, span));
                    } else {
                        return Err(RyndError::LexError {
                            message: "Expected '&&'".to_string(),
                            span,
                        });
                    }
                }
                '=' => {
                    self.advance();
                    if self.peek() == '=' {
                        self.advance();
                        tokens.push(Token::new(TokenType::EqualEqual, span));
                    } else if self.peek() == '>' {
                        self.advance();
                        tokens.push(Token::new(TokenType::FatArrow, span));
                    } else {
                        tokens.push(Token::new(TokenType::Equal, span));
                    }
                }
                '!' => {
                    self.advance();
                    if self.peek() == '=' {
                        self.advance();
                        tokens.push(Token::new(TokenType::BangEqual, span));
                    } else {
                        tokens.push(Token::new(TokenType::Bang, span));
                    }
                }
                '<' => {
                    self.advance();
                    if self.peek() == '=' {
                        self.advance();
                        tokens.push(Token::new(TokenType::LessEqual, span));
                    } else {
                        tokens.push(Token::new(TokenType::Less, span));
                    }
                }
                '>' => {
                    self.advance();
                    if self.peek() == '=' {
                        self.advance();
                        tokens.push(Token::new(TokenType::GreaterEqual, span));
                    } else {
                        tokens.push(Token::new(TokenType::Greater, span));
                    }
                }
                ':' => {
                    self.advance();
                    if self.peek() == ':' {
                        self.advance();
                        tokens.push(Token::new(TokenType::DoubleColon, span));
                    } else {
                        tokens.push(Token::new(TokenType::Colon, span));
                    }
                }
                '?' => {
                    self.advance();
                    if self.peek() == ':' {
                        self.advance();
                        tokens.push(Token::new(TokenType::Elvis, span));
                    } else if self.peek() == '.' {
                        self.advance();
                        tokens.push(Token::new(TokenType::SafeNav, span));
                    } else {
                        tokens.push(Token::new(TokenType::Question, span));
                    }
                }
                '.' => {
                    self.advance();
                    if self.peek() == '.' {
                        self.advance();
                        if self.peek() == '=' {
                            self.advance();
                            tokens.push(Token::new(TokenType::DotDotEqual, span));
                        } else {
                            tokens.push(Token::new(TokenType::DotDot, span));
                        }
                    } else {
                        tokens.push(Token::new(TokenType::Dot, span));
                    }
                }
                '"' => {
                    let str_tok = self.lex_string(span)?;
                    tokens.push(str_tok);
                }
                '0'..='9' => {
                    let num_tok = self.lex_number(span)?;
                    tokens.push(num_tok);
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    let ident_tok = self.lex_identifier(span);
                    tokens.push(ident_tok);
                }
                _ => {
                    return Err(RyndError::LexError {
                        message: format!("Unexpected character: '{ch}'"),
                        span,
                    });
                }
            }
        }

        if interpolation {
            return Err(RyndError::LexError {
                message: "Unterminated string interpolation".into(),
                span: Span::new(self.line, self.column),
            });
        }
        tokens.push(Token::new(
            TokenType::Eof,
            Span::new(self.line, self.column),
        ));
        Ok(tokens)
    }

    fn skip_whitespace_and_comments(&mut self) {
        while !self.is_at_end() {
            let ch = self.peek();
            if ch == ' ' || ch == '\t' || ch == '\r' || ch == '\n' {
                self.advance();
            } else if ch == '#' || (ch == '/' && self.peek_next() == '/') {
                // Comment until end of line
                while !self.is_at_end() && self.peek() != '\n' {
                    self.advance();
                }
            } else {
                break;
            }
        }
    }

    fn lex_string(&mut self, span: Span) -> RyndResult<Token> {
        self.advance(); // consume opening quote
        let mut value = String::new();
        let mut template = Vec::new();

        while !self.is_at_end() && self.peek() != '"' {
            let ch = self.peek();
            if ch == '#' && self.source[self.cursor..].starts_with("#{") {
                let part_span = Span::new(self.line, self.column);
                self.advance();
                self.advance();
                template.push(Token::new(
                    TokenType::StringLit(std::mem::take(&mut value)),
                    span.clone(),
                ));
                template.push(Token::new(TokenType::Plus, part_span.clone()));
                template.push(Token::new(TokenType::LParen, part_span.clone()));
                let mut expression = self.tokenize_inner(true)?;
                if expression.len() == 1 {
                    return Err(RyndError::LexError {
                        message: "Empty string interpolation".into(),
                        span: part_span,
                    });
                }
                expression.pop();
                template.extend(expression);
                template.push(Token::new(TokenType::RParen, part_span.clone()));
                template.push(Token::new(TokenType::Plus, part_span));
            } else if ch == '\\' {
                self.advance();
                if self.is_at_end() {
                    return Err(RyndError::LexError {
                        message: "Unterminated string escape".to_string(),
                        span,
                    });
                }
                match self.peek() {
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    '\\' => value.push('\\'),
                    '"' => value.push('"'),
                    '#' => value.push('#'),
                    other => {
                        value.push('\\');
                        value.push(other);
                    }
                }
                self.advance();
            } else {
                value.push(ch);
                self.advance();
            }
        }

        if self.is_at_end() {
            return Err(RyndError::LexError {
                message: "Unterminated string literal".to_string(),
                span,
            });
        }

        self.advance(); // consume closing quote
        if template.is_empty() {
            Ok(Token::new(TokenType::StringLit(value), span))
        } else {
            template.push(Token::new(TokenType::StringLit(value), span.clone()));
            Ok(Token::new(TokenType::Template(template), span))
        }
    }

    fn lex_number(&mut self, span: Span) -> RyndResult<Token> {
        let start_pos = self.cursor;
        while !self.is_at_end() && self.peek().is_ascii_digit() {
            self.advance();
        }

        let mut is_float = false;
        if !self.is_at_end() && self.peek() == '.' && self.peek_next().is_ascii_digit() {
            is_float = true;
            self.advance(); // consume '.'
            while !self.is_at_end() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }

        if matches!(self.peek(), 'e' | 'E') {
            is_float = true;
            self.advance();
            if matches!(self.peek(), '+' | '-') {
                self.advance();
            }
            if !self.peek().is_ascii_digit() {
                return Err(RyndError::LexError {
                    message: "Expected exponent digits".into(),
                    span,
                });
            }
            while self.peek().is_ascii_digit() {
                self.advance();
            }
        }
        let slice = self.extract_range(start_pos, self.cursor);
        if is_float {
            let val: f64 = slice.parse().map_err(|e| RyndError::LexError {
                message: format!("Invalid float '{slice}': {e}"),
                span: span.clone(),
            })?;
            Ok(Token::new(TokenType::Float(val), span))
        } else {
            if slice == "9223372036854775808" {
                return Ok(Token::new(TokenType::MinIntMagnitude, span));
            }
            let val: i64 = slice.parse().map_err(|e| RyndError::LexError {
                message: format!("Invalid integer '{slice}': {e}"),
                span: span.clone(),
            })?;
            Ok(Token::new(TokenType::Integer(val), span))
        }
    }

    fn lex_identifier(&mut self, span: Span) -> Token {
        let start_pos = self.cursor;
        while !self.is_at_end() && (self.peek().is_alphanumeric() || self.peek() == '_') {
            self.advance();
        }

        let ident = self.extract_range(start_pos, self.cursor);
        let token_type = match ident {
            "fn" => TokenType::Fn,
            "pub" => TokenType::Pub,
            "import" => TokenType::Import,
            "as" => TokenType::As,
            "unless" => TokenType::Unless,
            "let" => TokenType::Let,
            "mut" => TokenType::Mut,
            "if" => TokenType::If,
            "else" => TokenType::Else,
            "match" => TokenType::Match,
            "for" => TokenType::For,
            "in" => TokenType::In,
            "return" => TokenType::Return,
            "true" => TokenType::True,
            "false" => TokenType::False,
            "nil" => TokenType::Nil,
            "Some" => TokenType::Some,
            "None" => TokenType::None,
            "Ok" => TokenType::Ok,
            "Err" => TokenType::Err,
            "_" => TokenType::Underscore,
            _ => TokenType::Identifier(ident.to_string()),
        };

        Token::new(token_type, span)
    }

    fn peek(&self) -> char {
        self.source[self.cursor..].chars().next().unwrap_or('\0')
    }
    fn peek_next(&self) -> char {
        self.source[self.cursor..].chars().nth(1).unwrap_or('\0')
    }
    fn advance(&mut self) -> char {
        let ch = self.peek();
        if self.is_at_end() {
            return ch;
        }
        self.cursor += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        ch
    }
    fn is_at_end(&self) -> bool {
        self.cursor >= self.source.len()
    }
    fn extract_range(&self, start: usize, end: usize) -> &str {
        &self.source[start..end]
    }
}
