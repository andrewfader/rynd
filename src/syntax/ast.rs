use crate::error::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Nil,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Literal(Literal),
    Variable(String),
    Wildcard,
    Tuple(Vec<Pattern>),
    List(Vec<Pattern>),
    Variant { name: String, args: Vec<Pattern> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Literal(Literal),
    Identifier(String),
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Pipeline {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Lambda {
        params: Vec<String>,
        body: Box<Expr>,
    },
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Tuple(Vec<Expr>),
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
    },
    FieldAccess {
        target: Box<Expr>,
        field: String,
    },
    SafeFieldAccess {
        target: Box<Expr>,
        field: String,
    },
    Elvis {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
    },
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },
    Match {
        target: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    MapComprehension {
        key: Box<Expr>,
        value: Box<Expr>,
        variable: String,
        iterable: Box<Expr>,
        condition: Option<Box<Expr>>,
    },
    Comprehension {
        element: Box<Expr>,
        variable: String,
        iterable: Box<Expr>,
        condition: Option<Box<Expr>>,
    },
    Block {
        statements: Vec<Stmt>,
        final_expr: Option<Box<Expr>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Import {
        path: String,
        alias: String,
        span: Span,
    },
    Public(Box<Stmt>),
    Let {
        pattern: Pattern,
        init: Expr,
        is_mut: bool,
        span: Span,
    },
    Function {
        name: String,
        params: Vec<String>,
        body: Expr,
        span: Span,
    },
    Expression(Expr),
    Return {
        value: Option<Expr>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub statements: Vec<Stmt>,
}

/// Lower comprehensions identically for bytecode and native code generation.
pub fn lower_comprehension(expr: &Expr) -> Option<Expr> {
    let span = expr.span.clone();
    let make = |kind| Expr::new(kind, span.clone());
    let call = |name: &str, args| {
        make(ExprKind::Call {
            callee: Box::new(make(ExprKind::Identifier(name.into()))),
            args,
        })
    };
    let (element, variable, iterable, condition, is_map) = match &expr.kind {
        ExprKind::Comprehension {
            element,
            variable,
            iterable,
            condition,
        } => ((**element).clone(), variable, iterable, condition, false),
        ExprKind::MapComprehension {
            key,
            value,
            variable,
            iterable,
            condition,
        } => (
            make(ExprKind::Tuple(vec![(**key).clone(), (**value).clone()])),
            variable,
            iterable,
            condition,
            true,
        ),
        _ => return None,
    };
    let lambda = |body| {
        make(ExprKind::Lambda {
            params: vec![variable.clone()],
            body: Box::new(body),
        })
    };
    let items = if let Some(condition) = condition {
        call(
            "filter",
            vec![(**iterable).clone(), lambda((**condition).clone())],
        )
    } else {
        (**iterable).clone()
    };
    let mapped = call("map", vec![items, lambda(element)]);
    Some(if is_map {
        call("to_map", vec![mapped])
    } else {
        mapped
    })
}
