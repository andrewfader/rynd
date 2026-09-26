use rynd::syntax::ast::*;
use rynd::syntax::lexer::Lexer;
use rynd::syntax::parser::Parser;

fn parse_program(source: &str) -> Program {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize().expect("Lexing failed");
    let mut parser = Parser::new(tokens);
    parser.parse().expect("Parsing failed")
}

#[test]
fn test_parse_binary_precedence() {
    let prog = parse_program("1 + 2 * 3");
    assert_eq!(prog.statements.len(), 1);
    match &prog.statements[0] {
        Stmt::Expression(expr) => match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                assert_eq!(*op, BinaryOp::Add);
                assert_eq!(left.kind, ExprKind::Literal(Literal::Int(1)));
                match &right.kind {
                    ExprKind::Binary {
                        op: r_op,
                        left: r_left,
                        right: r_right,
                    } => {
                        assert_eq!(*r_op, BinaryOp::Mul);
                        assert_eq!(r_left.kind, ExprKind::Literal(Literal::Int(2)));
                        assert_eq!(r_right.kind, ExprKind::Literal(Literal::Int(3)));
                    }
                    _ => panic!("Expected multiplication on right"),
                }
            }
            _ => panic!("Expected binary expression"),
        },
        _ => panic!("Expected expression statement"),
    }
}

#[test]
fn test_parse_pipeline_expression() {
    let prog = parse_program("[1, 2] |> map(\\x -> x * 2)");
    assert_eq!(prog.statements.len(), 1);
    match &prog.statements[0] {
        Stmt::Expression(expr) => match &expr.kind {
            ExprKind::Pipeline { left, right } => {
                match &left.kind {
                    ExprKind::List(items) => assert_eq!(items.len(), 2),
                    _ => panic!("Expected list on left"),
                }
                match &right.kind {
                    ExprKind::Call { callee, args } => {
                        match &callee.kind {
                            ExprKind::Identifier(name) => assert_eq!(name, "map"),
                            _ => panic!("Expected identifier callee"),
                        }
                        assert_eq!(args.len(), 1);
                    }
                    _ => panic!("Expected function call on right of pipeline"),
                }
            }
            _ => panic!("Expected pipeline expression"),
        },
        _ => panic!("Expected expression statement"),
    }
}

#[test]
fn test_parse_comprehension() {
    let prog = parse_program("[x * 2 for x in numbers if x > 2]");
    assert_eq!(prog.statements.len(), 1);
    match &prog.statements[0] {
        Stmt::Expression(expr) => match &expr.kind {
            ExprKind::Comprehension {
                element,
                variable,
                iterable,
                condition,
            } => {
                assert_eq!(variable, "x");
                assert!(condition.is_some());
                match &iterable.kind {
                    ExprKind::Identifier(name) => assert_eq!(name, "numbers"),
                    _ => panic!("Expected identifier iterable"),
                }
                match &element.kind {
                    ExprKind::Binary { op, .. } => assert_eq!(*op, BinaryOp::Mul),
                    _ => panic!("Expected binary in element"),
                }
            }
            _ => panic!("Expected comprehension expression"),
        },
        _ => panic!("Expected expression statement"),
    }
}

#[test]
fn test_parse_function_and_let() {
    let source = r#"
        let x = 10
        fn square(n) {
            n * n
        }
    "#;
    let prog = parse_program(source);
    assert_eq!(prog.statements.len(), 2);

    match &prog.statements[0] {
        Stmt::Let {
            pattern,
            init,
            is_mut,
            ..
        } => {
            assert!(!is_mut);
            assert_eq!(*pattern, Pattern::Variable("x".to_string()));
            assert_eq!(init.kind, ExprKind::Literal(Literal::Int(10)));
        }
        _ => panic!("Expected let statement"),
    }

    match &prog.statements[1] {
        Stmt::Function {
            name, params, body, ..
        } => {
            assert_eq!(name, "square");
            assert_eq!(params, &["n".to_string()]);
            match &body.kind {
                ExprKind::Block {
                    statements,
                    final_expr,
                } => {
                    assert!(statements.is_empty());
                    assert!(final_expr.is_some());
                }
                _ => panic!("Expected block body"),
            }
        }
        _ => panic!("Expected function statement"),
    }
}

#[test]
fn test_parse_match_statement() {
    let source = r#"
        match result {
            Ok(v) => v + 1,
            Err(e) => 0,
            _ => -1
        }
    "#;
    let prog = parse_program(source);
    assert_eq!(prog.statements.len(), 1);

    match &prog.statements[0] {
        Stmt::Expression(expr) => match &expr.kind {
            ExprKind::Match { target, arms } => {
                match &target.kind {
                    ExprKind::Identifier(name) => assert_eq!(name, "result"),
                    _ => panic!("Expected target identifier"),
                }
                assert_eq!(arms.len(), 3);
                match &arms[0].pattern {
                    Pattern::Variant { name, args } => {
                        assert_eq!(name, "Ok");
                        assert_eq!(args.len(), 1);
                    }
                    _ => panic!("Expected Variant Ok pattern"),
                }
            }
            _ => panic!("Expected match expression"),
        },
        _ => panic!("Expected expression statement"),
    }
}

#[test]
fn test_parse_elvis_and_safe_nav() {
    let prog = parse_program("user?.name ?: \"Anonymous\"");
    assert_eq!(prog.statements.len(), 1);
    match &prog.statements[0] {
        Stmt::Expression(expr) => match &expr.kind {
            ExprKind::Elvis { left, right } => {
                match &left.kind {
                    ExprKind::SafeFieldAccess { target, field } => {
                        assert_eq!(field, "name");
                        match &target.kind {
                            ExprKind::Identifier(name) => assert_eq!(name, "user"),
                            _ => panic!("Expected target user"),
                        }
                    }
                    _ => panic!("Expected SafeFieldAccess on left"),
                }
                match &right.kind {
                    ExprKind::Literal(Literal::String(val)) => assert_eq!(val, "Anonymous"),
                    _ => panic!("Expected string literal on right"),
                }
            }
            _ => panic!("Expected Elvis expression"),
        },
        _ => panic!("Expected expression statement"),
    }
}
