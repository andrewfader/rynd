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
#[test]
fn test_parse_trailing_commas() {
    let prog = parse_program(
        r#"
        let (a, b,) = (1, 2,)
        let [c, d,] = [3, 4,]
        fn add(x, y,) { x + y }
        add(a, b,)
        match Some(1,) {
            Some(v,) => v,
            _ => 0,
        }
    "#,
    );
    assert_eq!(prog.statements.len(), 5);
}

#[test]
fn test_parse_prefix_and_unary() {
    let prog = parse_program("-x; !y; not z");
    assert_eq!(prog.statements.len(), 3);
    match &prog.statements[0] {
        Stmt::Expression(Expr {
            kind: ExprKind::Unary { op, operand },
            ..
        }) => {
            assert_eq!(*op, UnaryOp::Neg);
            assert_eq!(operand.kind, ExprKind::Identifier("x".into()));
        }
        _ => panic!("Expected unary negation"),
    }
    match &prog.statements[1] {
        Stmt::Expression(Expr {
            kind: ExprKind::Unary { op, operand },
            ..
        }) => {
            assert_eq!(*op, UnaryOp::Not);
            assert_eq!(operand.kind, ExprKind::Identifier("y".into()));
        }
        _ => panic!("Expected unary not"),
    }
    match &prog.statements[2] {
        Stmt::Expression(Expr {
            kind: ExprKind::Unary { op, operand },
            ..
        }) => {
            assert_eq!(*op, UnaryOp::Not);
            assert_eq!(operand.kind, ExprKind::Identifier("z".into()));
        }
        _ => panic!("Expected unary not keyword"),
    }
}

#[test]
fn test_parse_ternary_and_ranges() {
    let prog = parse_program("active ? 1 : 0; 1..10; 1..=10");
    assert_eq!(prog.statements.len(), 3);
    match &prog.statements[0] {
        Stmt::Expression(Expr {
            kind:
                ExprKind::If {
                    condition,
                    then_branch,
                    else_branch,
                },
            ..
        }) => {
            assert_eq!(condition.kind, ExprKind::Identifier("active".into()));
            assert_eq!(then_branch.kind, ExprKind::Literal(Literal::Int(1)));
            assert_eq!(
                else_branch.as_ref().unwrap().kind,
                ExprKind::Literal(Literal::Int(0))
            );
        }
        _ => panic!("Expected ternary parsed as If"),
    }
    match &prog.statements[1] {
        Stmt::Expression(Expr {
            kind:
                ExprKind::Range {
                    start,
                    end,
                    inclusive,
                },
            ..
        }) => {
            assert!(!inclusive);
            assert_eq!(start.kind, ExprKind::Literal(Literal::Int(1)));
            assert_eq!(end.kind, ExprKind::Literal(Literal::Int(10)));
        }
        _ => panic!("Expected exclusive range"),
    }
    match &prog.statements[2] {
        Stmt::Expression(Expr {
            kind:
                ExprKind::Range {
                    start,
                    end,
                    inclusive,
                },
            ..
        }) => {
            assert!(inclusive);
            assert_eq!(start.kind, ExprKind::Literal(Literal::Int(1)));
            assert_eq!(end.kind, ExprKind::Literal(Literal::Int(10)));
        }
        _ => panic!("Expected inclusive range"),
    }
}

#[test]
fn test_parse_indexing_and_calls() {
    let prog = parse_program("items[0]; foo(); bar(1, 2)");
    assert_eq!(prog.statements.len(), 3);
    match &prog.statements[0] {
        Stmt::Expression(Expr {
            kind: ExprKind::Index { target, index },
            ..
        }) => {
            assert_eq!(target.kind, ExprKind::Identifier("items".into()));
            assert_eq!(index.kind, ExprKind::Literal(Literal::Int(0)));
        }
        _ => panic!("Expected index expression"),
    }
    match &prog.statements[1] {
        Stmt::Expression(Expr {
            kind: ExprKind::Call { callee, args },
            ..
        }) => {
            assert_eq!(callee.kind, ExprKind::Identifier("foo".into()));
            assert!(args.is_empty());
        }
        _ => panic!("Expected zero-arg call"),
    }
    match &prog.statements[2] {
        Stmt::Expression(Expr {
            kind: ExprKind::Call { callee, args },
            ..
        }) => {
            assert_eq!(callee.kind, ExprKind::Identifier("bar".into()));
            assert_eq!(args.len(), 2);
        }
        _ => panic!("Expected two-arg call"),
    }
}

#[test]
fn test_parse_lambdas_and_destructuring() {
    let prog = parse_program(r#"\x, y -> x + y; |x, y| x * y; \ -> 42; \(a, b) -> a + b"#);
    assert_eq!(prog.statements.len(), 4);

    match &prog.statements[0] {
        Stmt::Expression(Expr {
            kind: ExprKind::Lambda { params, body },
            ..
        }) => {
            assert_eq!(params, &["x", "y"]);
            assert!(matches!(body.kind, ExprKind::Binary { .. }));
        }
        _ => panic!("Expected backslash lambda"),
    }
    match &prog.statements[1] {
        Stmt::Expression(Expr {
            kind: ExprKind::Lambda { params, body },
            ..
        }) => {
            assert_eq!(params, &["x", "y"]);
            assert!(matches!(body.kind, ExprKind::Binary { .. }));
        }
        _ => panic!("Expected pipe lambda"),
    }
    match &prog.statements[2] {
        Stmt::Expression(Expr {
            kind: ExprKind::Lambda { params, body },
            ..
        }) => {
            assert!(params.is_empty());
            assert_eq!(body.kind, ExprKind::Literal(Literal::Int(42)));
        }
        _ => panic!("Expected zero-arg lambda"),
    }
    match &prog.statements[3] {
        Stmt::Expression(Expr {
            kind: ExprKind::Lambda { params, body },
            ..
        }) => {
            assert_eq!(params, &["@argument0"]);
            match &body.kind {
                ExprKind::Block { statements, .. } => {
                    assert_eq!(statements.len(), 1);
                }
                _ => panic!("Expected destructuring block body"),
            }
        }
        _ => panic!("Expected parameter destructuring lambda"),
    }
}

#[test]
fn test_parse_if_and_unless_expressions() {
    let prog = parse_program(
        r#"
        if c1 { 1 } else { 2 }
        unless c2 { 3 } else { 4 }
        if c1 { 1 } else if c2 { 2 } else { 3 }
        if c1 { 1 } else unless c2 { 2 } else { 3 }
    "#,
    );
    assert_eq!(prog.statements.len(), 4);
}

#[test]
fn test_parse_statement_modifiers() {
    let prog = parse_program(
        r#"
        process() if active
        increment() unless disabled
    "#,
    );
    assert_eq!(prog.statements.len(), 2);
    match &prog.statements[0] {
        Stmt::Expression(Expr {
            kind:
                ExprKind::If {
                    condition,
                    then_branch,
                    ..
                },
            ..
        }) => {
            assert_eq!(condition.kind, ExprKind::Identifier("active".into()));
            assert!(matches!(then_branch.kind, ExprKind::Call { .. }));
        }
        _ => panic!("Expected statement modifier if"),
    }
}

#[test]
fn test_parse_maps_and_map_comprehensions() {
    let prog = parse_program(
        r#"
        let empty = {}
        let user = {"name": "Alice", "age": 30}
        let double_map = {k: k * 2 for k in numbers if k > 0}
    "#,
    );
    assert_eq!(prog.statements.len(), 3);
    match &prog.statements[0] {
        Stmt::Let { init, .. } => match &init.kind {
            ExprKind::Map(items) => assert!(items.is_empty()),
            _ => panic!("Expected empty map"),
        },
        _ => panic!("Expected let statement"),
    }
    match &prog.statements[1] {
        Stmt::Let { init, .. } => match &init.kind {
            ExprKind::Map(items) => assert_eq!(items.len(), 2),
            _ => panic!("Expected map with 2 entries"),
        },
        _ => panic!("Expected let statement"),
    }
    match &prog.statements[2] {
        Stmt::Let { init, .. } => match &init.kind {
            ExprKind::MapComprehension {
                variable,
                condition,
                ..
            } => {
                assert_eq!(variable, "k");
                assert!(condition.is_some());
            }
            _ => panic!("Expected map comprehension"),
        },
        _ => panic!("Expected let statement"),
    }
}

#[test]
fn test_parse_imports_and_visibility() {
    let prog = parse_program(
        r#"
        import "math.rynd" as math
        pub fn square(x) { x * x }
        pub let rate = 1.05
    "#,
    );
    assert_eq!(prog.statements.len(), 3);
    match &prog.statements[0] {
        Stmt::Import { path, alias, .. } => {
            assert_eq!(path, "math.rynd");
            assert_eq!(alias, "math");
        }
        _ => panic!("Expected import statement"),
    }
    match &prog.statements[1] {
        Stmt::Public(inner) => match inner.as_ref() {
            Stmt::Function { name, .. } => assert_eq!(name, "square"),
            _ => panic!("Expected function inside public"),
        },
        _ => panic!("Expected public statement"),
    }
    match &prog.statements[2] {
        Stmt::Public(inner) => match inner.as_ref() {
            Stmt::Let { pattern, .. } => match pattern {
                Pattern::Variable(name) => assert_eq!(name, "rate"),
                _ => panic!("Expected variable pattern"),
            },
            _ => panic!("Expected let inside public"),
        },
        _ => panic!("Expected public statement"),
    }
}

#[test]
fn test_parse_match_guards_and_patterns() {
    let prog = parse_program(
        r#"
        match val {
            0 => "zero",
            n if n > 0 => "positive",
            (x, y) => "tuple",
            [a, b] => "list",
            _ => "other"
        }
    "#,
    );
    assert_eq!(prog.statements.len(), 1);
    match &prog.statements[0] {
        Stmt::Expression(Expr {
            kind: ExprKind::Match { arms, .. },
            ..
        }) => {
            assert_eq!(arms.len(), 5);
            assert!(arms[1].guard.is_some());
        }
        _ => panic!("Expected match expression"),
    }
}

#[test]
fn test_parse_errors() {
    fn parse_err(source: &str) -> String {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("Lexing failed");
        let mut parser = Parser::new(tokens);
        parser.parse().unwrap_err().to_string()
    }

    assert!(parse_err("pub 123").contains("pub must precede fn or let"));
    assert!(parse_err("import 123 as foo").contains("Expected a literal module path"));
    assert!(parse_err("import \"math.rynd\" foo").contains("Expected 'as'"));
    assert!(parse_err("let x").contains("Expected '='"));
    assert!(parse_err("fn (x) {}").contains("Expected function name"));
    assert!(parse_err("fn add(a, a) {}").contains("Duplicate function parameter"));
    assert!(parse_err(r#"\a, a -> a"#).contains("Duplicate function parameter"));
    assert!(parse_err("items[0").contains("Expected ']'"));
    assert!(parse_err("obj.").contains("Expected field name after '.'"));
    assert!(parse_err("obj?.").contains("Expected field name after '?.'"));
}
