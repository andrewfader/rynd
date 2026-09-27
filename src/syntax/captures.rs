//! Lexical free-name analysis shared by both compilation targets.
use super::ast::*;
use crate::vm::runtime::pattern_names;
use std::collections::HashSet;

pub fn free_names(name: &str, params: &[String], body: &Expr) -> HashSet<String> {
    let mut bound = params.to_vec();
    bound.push(name.into());
    let mut free = HashSet::new();
    expression(body, &mut bound, &mut free);
    free
}

fn statement(stmt: &Stmt, bound: &mut Vec<String>, free: &mut HashSet<String>) {
    match stmt {
        Stmt::Public(inner) => statement(inner, bound, free),
        Stmt::Import { alias, .. } => bound.push(alias.clone()),
        Stmt::Let { pattern, init, .. } => {
            expression(init, bound, free);
            bound.extend(pattern_names(pattern));
        }
        Stmt::Function {
            name, params, body, ..
        } => {
            for used in free_names(name, params, body) {
                if !bound.contains(&used) {
                    free.insert(used);
                }
            }
            bound.push(name.clone());
        }
        Stmt::Expression(expr) => expression(expr, bound, free),
        Stmt::Return { value, .. } => {
            if let Some(value) = value {
                expression(value, bound, free);
            }
        }
    }
}

fn expression(expr: &Expr, bound: &mut Vec<String>, free: &mut HashSet<String>) {
    // Include the builtin names introduced by comprehension lowering, too:
    // users can shadow map/filter/to_map with local functions.
    if let Some(lowered) = lower_comprehension(expr) {
        expression(&lowered, bound, free);
        return;
    }
    let scope = bound.len();
    match &expr.kind {
        ExprKind::Literal(_) => {}
        ExprKind::Identifier(name) => {
            if !bound.contains(name) {
                free.insert(name.clone());
            }
        }
        ExprKind::Binary { left, right, .. }
        | ExprKind::Pipeline { left, right }
        | ExprKind::Elvis { left, right }
        | ExprKind::Range {
            start: left,
            end: right,
            ..
        }
        | ExprKind::Index {
            target: left,
            index: right,
        } => {
            expression(left, bound, free);
            expression(right, bound, free);
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::FieldAccess {
            target: operand, ..
        }
        | ExprKind::SafeFieldAccess {
            target: operand, ..
        } => expression(operand, bound, free),
        ExprKind::Call { callee, args } => {
            expression(callee, bound, free);
            for arg in args {
                expression(arg, bound, free);
            }
        }
        ExprKind::Lambda { params, body } => {
            bound.extend(params.iter().cloned());
            expression(body, bound, free);
        }
        ExprKind::List(items) | ExprKind::Tuple(items) => {
            for item in items {
                expression(item, bound, free);
            }
        }
        ExprKind::Map(items) => {
            for (key, value) in items {
                expression(key, bound, free);
                expression(value, bound, free);
            }
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expression(condition, bound, free);
            expression(then_branch, bound, free);
            if let Some(branch) = else_branch {
                expression(branch, bound, free);
            }
        }
        ExprKind::Match { target, arms } => {
            expression(target, bound, free);
            for arm in arms {
                bound.extend(pattern_names(&arm.pattern));
                if let Some(guard) = &arm.guard {
                    expression(guard, bound, free);
                }
                expression(&arm.body, bound, free);
                bound.truncate(scope);
            }
        }
        ExprKind::Block {
            statements,
            final_expr,
        } => {
            for stmt in statements {
                statement(stmt, bound, free);
            }
            if let Some(expr) = final_expr {
                expression(expr, bound, free);
            }
        }
        ExprKind::Comprehension { .. } | ExprKind::MapComprehension { .. } => unreachable!(),
    }
    bound.truncate(scope);
}
