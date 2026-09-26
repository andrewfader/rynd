//! Resolve a file graph once and qualify module globals before either backend.
use crate::syntax::{
    ast::*,
    lexer::Lexer,
    parser::Parser,
    token::{Token, TokenType},
};
use crate::vm::runtime::pattern_names;
use crate::{RyndError, RyndResult, Span};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};

pub struct ModuleBundle {
    pub program: Program,
    pub files: Vec<PathBuf>,
    pub exports: Vec<String>,
}

pub fn load(path: impl AsRef<Path>) -> RyndResult<ModuleBundle> {
    let mut loader = Loader::default();
    loader.module(path.as_ref(), true)?;
    Ok(ModuleBundle {
        program: Program {
            statements: loader.statements,
        },
        files: loader.files,
        exports: loader.exports,
    })
}

#[derive(Default)]
struct Loader {
    loaded: HashMap<PathBuf, String>,
    active: Vec<PathBuf>,
    files: Vec<PathBuf>,
    statements: Vec<Stmt>,
    exports: Vec<String>,
}

fn failure(message: impl Into<String>, span: &Span) -> RyndError {
    RyndError::CompileError {
        message: message.into(),
    }
    .at(span.clone())
}

fn in_file(tokens: &mut [Token], file: &Rc<str>) {
    for token in tokens {
        token.span.file = Some(file.clone());
        if let TokenType::Template(parts) = &mut token.token_type {
            in_file(parts, file);
        }
    }
}

fn declaration(stmt: &Stmt) -> Vec<String> {
    match stmt {
        Stmt::Public(inner) => declaration(inner),
        Stmt::Function { name, .. } => vec![name.clone()],
        Stmt::Let { pattern, .. } => pattern_names(pattern),
        Stmt::Import { alias, .. } => vec![alias.clone()],
        _ => vec![],
    }
}

impl Loader {
    fn module(&mut self, path: &Path, root: bool) -> RyndResult<String> {
        let path = path
            .canonicalize()
            .map_err(|e| RyndError::IoError(format!("{}: {e}", path.display())))?;
        if self.active.contains(&path) {
            let chain = self
                .active
                .iter()
                .chain(std::iter::once(&path))
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(RyndError::CompileError {
                message: format!("Cyclic module imports: {chain}"),
            });
        }
        if let Some(name) = self.loaded.get(&path) {
            return Ok(name.clone());
        }
        if self.active.len() >= 128 {
            return Err(RyndError::CompileError {
                message: "Module import depth exceeds 128".into(),
            });
        }
        let namespace = format!("@module:{}", path.display());
        self.files.push(path.clone());
        self.active.push(path.clone());
        let source = std::fs::read_to_string(&path)
            .map_err(|e| RyndError::IoError(format!("{}: {e}", path.display())))?;
        let file: Rc<str> = path.to_string_lossy().as_ref().into();
        let mut tokens = Lexer::new(&source)
            .tokenize()
            .map_err(|e| e.with_file(file.clone()))?;
        in_file(&mut tokens, &file);
        let mut program = Parser::new(tokens).parse()?;
        let names: HashMap<String, String> = program
            .statements
            .iter()
            .flat_map(declaration)
            .map(|name| {
                let qualified = if root {
                    name.clone()
                } else {
                    format!("{namespace}::{name}")
                };
                (name, qualified)
            })
            .collect();
        let mut exports = Vec::new();
        let mut imports = HashSet::new();
        for statement in &mut program.statements {
            if let Stmt::Import {
                path: relative,
                alias,
                span,
            } = statement
            {
                if !imports.insert(alias.clone()) {
                    return Err(failure(format!("Duplicate import alias '{alias}'"), span));
                }
                let imported = self
                    .module(&path.parent().unwrap().join(relative), false)
                    .map_err(|e| e.at(span.clone()))?;
                *statement = Stmt::Let {
                    pattern: Pattern::Variable(alias.clone()),
                    init: Expr::new(ExprKind::Identifier(imported), span.clone()),
                    is_mut: false,
                    span: span.clone(),
                };
            }
            if let Stmt::Public(inner) = statement {
                for name in declaration(inner) {
                    if exports.contains(&name) {
                        return Err(RyndError::CompileError {
                            message: format!("Duplicate export '{name}' in {}", path.display()),
                        });
                    }
                    exports.push(name);
                }
                *statement = *inner.clone();
            }
            qualify_statement(statement, &names, &mut HashSet::new(), true);
        }
        if root {
            self.exports = exports;
        } else {
            let span = Span::new(1, 1).with_file(file);
            let fields = exports
                .into_iter()
                .map(|name| {
                    (
                        Expr::new(
                            ExprKind::Literal(Literal::String(name.clone())),
                            span.clone(),
                        ),
                        Expr::new(ExprKind::Identifier(names[&name].clone()), span.clone()),
                    )
                })
                .collect();
            program.statements.push(Stmt::Let {
                pattern: Pattern::Variable(namespace.clone()),
                init: Expr::new(ExprKind::Map(fields), span.clone()),
                is_mut: false,
                span,
            });
        }
        self.statements.extend(program.statements);
        self.active.pop();
        self.loaded.insert(path, namespace.clone());
        Ok(namespace)
    }
}

fn qualify_pattern(pattern: &mut Pattern, names: &HashMap<String, String>) {
    match pattern {
        Pattern::Variable(name) => {
            if let Some(qualified) = names.get(name) {
                *name = qualified.clone();
            }
        }
        Pattern::Tuple(items) | Pattern::List(items) | Pattern::Variant { args: items, .. } => {
            for item in items {
                qualify_pattern(item, names);
            }
        }
        _ => {}
    }
}

fn qualify_statement(
    stmt: &mut Stmt,
    names: &HashMap<String, String>,
    locals: &mut HashSet<String>,
    top: bool,
) {
    match stmt {
        Stmt::Let { pattern, init, .. } => {
            qualify_expr(init, names, locals);
            if top {
                qualify_pattern(pattern, names);
            } else {
                locals.extend(pattern_names(pattern));
            }
        }
        Stmt::Function {
            name, params, body, ..
        } => {
            if top {
                *name = names[name].clone();
            } else {
                locals.insert(name.clone());
            }
            let mut inner = locals.clone();
            inner.extend(params.iter().cloned());
            qualify_expr(body, names, &inner);
        }
        Stmt::Expression(expr) => qualify_expr(expr, names, locals),
        Stmt::Return {
            value: Some(expr), ..
        } => qualify_expr(expr, names, locals),
        // Nested public/import statements are rejected by the compiler.
        _ => {}
    }
}

fn qualify_expr(expr: &mut Expr, names: &HashMap<String, String>, locals: &HashSet<String>) {
    if let Some(lowered) = lower_comprehension(expr) {
        *expr = lowered;
    }
    match &mut expr.kind {
        ExprKind::Identifier(name) => {
            if !locals.contains(name)
                && let Some(qualified) = names.get(name)
            {
                *name = qualified.clone();
            }
        }
        ExprKind::Binary { left, right, .. }
        | ExprKind::Pipeline { left, right }
        | ExprKind::Elvis { left, right } => {
            qualify_expr(left, names, locals);
            qualify_expr(right, names, locals);
        }
        ExprKind::Unary { operand, .. } => qualify_expr(operand, names, locals),
        ExprKind::Call { callee, args } => {
            qualify_expr(callee, names, locals);
            for arg in args {
                qualify_expr(arg, names, locals);
            }
        }
        ExprKind::Lambda { params, body } => {
            let mut inner = locals.clone();
            inner.extend(params.iter().cloned());
            qualify_expr(body, names, &inner);
        }
        ExprKind::List(items) | ExprKind::Tuple(items) => {
            for item in items {
                qualify_expr(item, names, locals);
            }
        }
        ExprKind::Map(items) => {
            for (key, value) in items {
                qualify_expr(key, names, locals);
                qualify_expr(value, names, locals);
            }
        }
        ExprKind::Index { target, index } => {
            qualify_expr(target, names, locals);
            qualify_expr(index, names, locals);
        }
        ExprKind::FieldAccess { target, .. } | ExprKind::SafeFieldAccess { target, .. } => {
            qualify_expr(target, names, locals)
        }
        ExprKind::Range { start, end, .. } => {
            qualify_expr(start, names, locals);
            qualify_expr(end, names, locals);
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            qualify_expr(condition, names, locals);
            qualify_expr(then_branch, names, locals);
            if let Some(branch) = else_branch {
                qualify_expr(branch, names, locals);
            }
        }
        ExprKind::Match { target, arms } => {
            qualify_expr(target, names, locals);
            for arm in arms {
                let mut inner = locals.clone();
                inner.extend(pattern_names(&arm.pattern));
                if let Some(guard) = &mut arm.guard {
                    qualify_expr(guard, names, &inner);
                }
                qualify_expr(&mut arm.body, names, &inner);
            }
        }
        ExprKind::Block {
            statements,
            final_expr,
        } => {
            let mut inner = locals.clone();
            for statement in statements {
                qualify_statement(statement, names, &mut inner, false);
            }
            if let Some(expr) = final_expr {
                qualify_expr(expr, names, &inner);
            }
        }
        ExprKind::Literal(_) => {}
        ExprKind::Comprehension { .. } | ExprKind::MapComprehension { .. } => {
            unreachable!("comprehensions were lowered")
        }
    }
}
