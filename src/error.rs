use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
    pub file: Option<std::rc::Rc<str>>,
}

impl Span {
    pub fn new(line: usize, column: usize) -> Self {
        Self {
            line,
            column,
            file: None,
        }
    }
    pub fn with_file(mut self, file: impl Into<std::rc::Rc<str>>) -> Self {
        self.file = Some(file.into());
        self
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{file}:")?;
        }
        write!(f, "{}:{}", self.line, self.column)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RyndError {
    Located { span: Span, source: Box<RyndError> },
    LexError { message: String, span: Span },
    ParseError { message: String, span: Span },
    CompileError { message: String },
    RuntimeError { message: String },
    TranspileError { message: String },
    IoError(String),
}

impl fmt::Display for RyndError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RyndError::Located { span, source } => write!(f, "{source} (at {span})"),
            RyndError::LexError { message, span } => {
                write!(f, "Lexer error at {span}: {message}")
            }
            RyndError::ParseError { message, span } => {
                write!(f, "Parse error at {span}: {message}")
            }
            RyndError::CompileError { message } => {
                write!(f, "Compile error: {message}")
            }
            RyndError::RuntimeError { message } => {
                write!(f, "Runtime error: {message}")
            }
            RyndError::TranspileError { message } => {
                write!(f, "Transpile error: {message}")
            }
            RyndError::IoError(msg) => write!(f, "IO error: {msg}"),
        }
    }
}

impl std::error::Error for RyndError {}

impl RyndError {
    pub fn with_file(mut self, file: impl Into<std::rc::Rc<str>>) -> Self {
        match &mut self {
            Self::Located { span, .. }
            | Self::LexError { span, .. }
            | Self::ParseError { span, .. } => span.file = Some(file.into()),
            _ => {}
        }
        self
    }
    pub fn at(self, span: Span) -> Self {
        match self {
            Self::Located { .. } | Self::LexError { .. } | Self::ParseError { .. } => self,
            _ => Self::Located {
                span,
                source: Box::new(self),
            },
        }
    }
}

pub type RyndResult<T> = Result<T, RyndError>;
