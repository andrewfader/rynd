//! Value semantics shared by bytecode execution and generated native Rust.
use super::value::Value;
use crate::error::{RyndError, RyndResult};
use crate::syntax::ast::{BinaryOp, Literal, Pattern, UnaryOp};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::io::{Read, Write};

pub fn error(message: impl Into<String>) -> RyndError {
    RyndError::RuntimeError {
        message: message.into(),
    }
}

pub trait Runtime {
    fn call(&mut self, callee: Value, args: Vec<Value>) -> RyndResult<Value>;
    fn call_ref(&mut self, callee: &Value, args: &[Value]) -> RyndResult<Value> {
        self.call(callee.clone(), args.to_vec())
    }
    fn write(&mut self, text: &str) -> RyndResult<()>;
    fn get_global(&self, name: &str) -> RyndResult<Value>;
    fn set_global(&mut self, name: &str, value: Value);
}

pub fn globals() -> HashMap<String, Value> {
    let mut globals = HashMap::new();
    for (name, arity) in [
        ("parse_json", 1),
        ("to_json", 1),
        ("attempt", 2),
        ("sort", 1),
        ("sort_by", 2),
        ("group_by", 2),
        ("keys", 1),
        ("values", 1),
        ("entries", 1),
        ("take", 2),
        ("skip", 2),
        ("any", 2),
        ("all", 2),
        ("find", 2),
        ("filter_map", 2),
        ("flat_map", 2),
        ("enumerate", 1),
        ("zip", 2),
        ("zip_with", 3),
        ("chunks", 2),
        ("windows", 2),
        ("reverse", 1),
        ("uniq", 1),
        ("flatten", 1),
        ("each", 2),
        ("partition", 2),
        ("scan", 3),
        ("map", 2),
        ("filter", 2),
        ("reduce", 3),
        ("sum", 1),
        ("len", 1),
        ("range", 2),
        ("head", 1),
        ("tail", 1),
        ("push", 2),
        ("concat", 2),
        ("print", 1),
        ("println", 1),
        ("to_string", 1),
        ("lines", 1),
        ("split", 2),
        ("trim", 1),
        ("join", 2),
        ("parse_int", 1),
        ("parse_float", 1),
        ("contains", 2),
        ("read_text", 1),
        ("read_stdin", 0),
        ("abs", 1),
        ("min", 2),
        ("max", 2),
        ("to_map", 1),
        ("Some", 1),
        ("Ok", 1),
        ("Err", 1),
    ]
    .iter()
    .copied()
    .chain(super::scripting::BUILTINS.iter().copied())
    {
        globals.insert(
            name.into(),
            Value::Builtin {
                name: name.into(),
                arity,
            },
        );
    }
    globals.insert("None".into(), Value::variant("None", vec![]));
    globals.insert("args".into(), Value::list(vec![]));
    globals
}

pub fn literal(lit: &Literal) -> Value {
    match lit {
        Literal::Nil => Value::Nil,
        Literal::Bool(v) => Value::Bool(*v),
        Literal::Int(v) => Value::Int(*v),
        Literal::Float(v) => Value::Float(*v),
        Literal::String(v) => Value::string(v.clone()),
    }
}

/// Compare without rounding large integers to f64 first.
pub fn compare_int_float(i: i64, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f >= 9223372036854775808.0 {
        return Some(Ordering::Less);
    }
    if f < -9223372036854775808.0 {
        return Some(Ordering::Greater);
    }
    let whole = f as i64;
    Some(
        i.cmp(&whole)
            .then_with(|| 0.0_f64.partial_cmp(&f.fract()).unwrap()),
    )
}

pub fn compare(a: &Value, b: &Value) -> RyndResult<Option<Ordering>> {
    Ok(match (a, b) {
        (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
        (Value::Int(a), Value::Float(b)) => compare_int_float(*a, *b),
        (Value::Float(a), Value::Int(b)) => compare_int_float(*b, *a).map(Ordering::reverse),
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        _ => {
            return Err(error(format!(
                "Cannot compare {} and {}",
                a.type_name(),
                b.type_name()
            )));
        }
    })
}

pub fn binary(op: &BinaryOp, a: Value, b: Value) -> RyndResult<Value> {
    use BinaryOp::*;
    if matches!(op, Equal | NotEqual) {
        return Ok(Value::Bool(if *op == Equal { a == b } else { a != b }));
    }
    if matches!(op, Less | LessEqual | Greater | GreaterEqual) {
        let order = compare(&a, &b)?;
        return Ok(Value::Bool(match op {
            Less => order == Some(Ordering::Less),
            Greater => order == Some(Ordering::Greater),
            LessEqual => matches!(order, Some(Ordering::Less | Ordering::Equal)),
            _ => matches!(order, Some(Ordering::Greater | Ordering::Equal)),
        }));
    }
    if *op == Add && (matches!(a, Value::String(_)) || matches!(b, Value::String(_))) {
        return Ok(Value::string(format!("{a}{b}")));
    }
    if let (Mul, Value::String(s), Value::Int(n)) = (op, &a, &b) {
        let len = s
            .len()
            .checked_mul((*n).max(0) as usize)
            .ok_or_else(|| error("String size overflow"))?;
        let mut out = String::new();
        out.try_reserve(len)
            .map_err(|_| error("String allocation failed"))?;
        for _ in 0..(*n).max(0) {
            out.push_str(s);
        }
        return Ok(Value::string(out));
    }
    match (&a, &b) {
        (Value::Int(a), Value::Int(b)) => {
            if matches!(op, Div | Mod) && *b == 0 {
                return Err(error(if *op == Div {
                    "Division by zero"
                } else {
                    "Modulo by zero"
                }));
            }
            let n = match op {
                Add => a.checked_add(*b),
                Sub => a.checked_sub(*b),
                Mul => a.checked_mul(*b),
                Div => a.checked_div(*b),
                Mod => a.checked_rem(*b),
                _ => None,
            }
            .ok_or_else(|| error("Integer overflow"))?;
            Ok(Value::Int(n))
        }
        (Value::Int(_) | Value::Float(_), Value::Int(_) | Value::Float(_)) if *op != Mod => {
            let a = number(&a)?;
            let b = number(&b)?;
            Ok(Value::Float(match op {
                Add => a + b,
                Sub => a - b,
                Mul => a * b,
                Div => a / b,
                _ => return Err(error("Invalid numeric operator")),
            }))
        }
        _ => Err(error(format!(
            "Cannot apply {op:?} to {} and {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

fn number(v: &Value) -> RyndResult<f64> {
    match v {
        Value::Int(i) => Ok(*i as f64),
        Value::Float(f) => Ok(*f),
        _ => Err(error("Expected number")),
    }
}

pub fn unary(op: &UnaryOp, value: Value) -> RyndResult<Value> {
    match (op, value) {
        (UnaryOp::Not, v) => Ok(Value::Bool(!v.is_truthy())),
        (UnaryOp::Neg, Value::Int(i)) => i
            .checked_neg()
            .map(Value::Int)
            .ok_or_else(|| error("Integer overflow")),
        (UnaryOp::Neg, Value::Float(f)) => Ok(Value::Float(-f)),
        (_, v) => Err(error(format!("Cannot negate {}", v.type_name()))),
    }
}

pub fn make_range(start: &Value, end: &Value, inclusive: bool) -> RyndResult<Value> {
    if let (Value::Int(start), Value::Int(end)) = (start, end) {
        let length = ((*end as i128) - (*start as i128) + i128::from(inclusive)).max(0);
        let length = usize::try_from(length).map_err(|_| error("Range size overflow"))?;
        let mut items = Vec::new();
        items
            .try_reserve(length)
            .map_err(|_| error("Range allocation failed"))?;
        if inclusive {
            items.extend((*start..=*end).map(Value::Int));
        } else {
            items.extend((*start..*end).map(Value::Int));
        }
        Ok(Value::list(items))
    } else {
        Err(error("range() expects integer bounds"))
    }
}

pub fn map_key(value: &Value) -> String {
    value.to_string()
}

pub fn index(target: &Value, index: &Value) -> RyndResult<Value> {
    fn normalize(i: i64, len: usize) -> Option<usize> {
        if i < 0 {
            len.checked_sub(i.unsigned_abs() as usize)
        } else {
            usize::try_from(i).ok()
        }
    }
    Ok(match (target, index) {
        (Value::List(items) | Value::Tuple(items), Value::Int(i)) => normalize(*i, items.len())
            .and_then(|n| items.get(n))
            .cloned()
            .unwrap_or(Value::Nil),
        (Value::String(s), Value::Int(i)) => normalize(*i, s.chars().count())
            .and_then(|n| s.chars().nth(n))
            .map(|c| Value::string(c.to_string()))
            .unwrap_or(Value::Nil),
        (Value::Map(m), Value::String(k)) => m.get(k.as_str()).cloned().unwrap_or(Value::Nil),
        _ => {
            return Err(error(format!(
                "Cannot index {} with {}",
                target.type_name(),
                index.type_name()
            )));
        }
    })
}

pub fn field(target: &Value, field: &str, safe: bool) -> RyndResult<Value> {
    Ok(match target {
        Value::Map(m) => m.get(field).cloned().unwrap_or(Value::Nil),
        Value::Variant { name, values } => match field {
            "tag" | "name" => Value::string(name.clone()),
            "value" if values.len() == 1 => values[0].clone(),
            _ => Value::Nil,
        },
        _ if safe => Value::Nil,
        _ => {
            return Err(error(format!(
                "Cannot access field '{field}' on {}",
                target.type_name()
            )));
        }
    })
}

/// Bindings are returned in pattern traversal order; mismatches expose no bindings.
pub fn match_pattern(pattern: &Pattern, value: &Value) -> Option<Vec<Value>> {
    fn walk(p: &Pattern, v: &Value, bindings: &mut Vec<Value>) -> bool {
        match (p, v) {
            (Pattern::Wildcard, _) => true,
            (Pattern::Variable(_), v) => {
                bindings.push(v.clone());
                true
            }
            (Pattern::Literal(l), v) => literal(l) == *v,
            (Pattern::Tuple(ps), Value::Tuple(vs)) | (Pattern::List(ps), Value::List(vs)) => {
                ps.len() == vs.len() && ps.iter().zip(vs.iter()).all(|(p, v)| walk(p, v, bindings))
            }
            (Pattern::Variant { name, args }, Value::Variant { name: tag, values }) => {
                name == tag
                    && args.len() == values.len()
                    && args
                        .iter()
                        .zip(values.iter())
                        .all(|(p, v)| walk(p, v, bindings))
            }
            _ => false,
        }
    }
    let mut bindings = Vec::new();
    walk(pattern, value, &mut bindings).then_some(bindings)
}

pub fn pattern_names(pattern: &Pattern) -> Vec<String> {
    match pattern {
        Pattern::Variable(name) => vec![name.clone()],
        Pattern::List(ps) | Pattern::Tuple(ps) | Pattern::Variant { args: ps, .. } => {
            ps.iter().flat_map(pattern_names).collect()
        }
        _ => vec![],
    }
}

pub fn call_builtin(rt: &mut dyn Runtime, name: &str, args: &[Value]) -> RyndResult<Value> {
    fn string(v: &Value) -> RyndResult<&str> {
        match v {
            Value::String(s) => Ok(s.as_str()),
            _ => Err(error(format!("Expected string, found {}", v.type_name()))),
        }
    }
    fn list(v: &Value) -> RyndResult<&[Value]> {
        if let Value::List(l) = v {
            Ok(l)
        } else {
            Err(error(format!("Expected list, found {}", v.type_name())))
        }
    }
    match name {
        "parse_json" => super::json::parse(string(&args[0])?),
        "to_json" => super::json::stringify(&args[0]).map(Value::string),
        "attempt" => match rt.call_ref(&args[0], list(&args[1])?) {
            Ok(value) => Ok(Value::variant("Ok", vec![value])),
            Err(error) => Ok(Value::variant(
                "Err",
                vec![Value::string(error.to_string())],
            )),
        },
        "sort" | "sort_by" | "group_by" | "keys" | "values" | "entries" | "take" | "skip"
        | "any" | "all" | "find" | "filter_map" | "flat_map" | "enumerate" | "zip" | "zip_with"
        | "chunks" | "windows" | "reverse" | "uniq" | "flatten" | "each" | "partition" | "scan" => {
            super::collections::call(rt, name, args)
        }
        "lines" => Ok(Value::list(
            string(&args[0])?.lines().map(Value::string).collect(),
        )),
        "split" => Ok(Value::list(
            string(&args[0])?
                .split(string(&args[1])?)
                .map(Value::string)
                .collect(),
        )),
        "trim" => Ok(Value::string(string(&args[0])?.trim())),
        "join" => {
            let parts = list(&args[0])?
                .iter()
                .map(string)
                .collect::<RyndResult<Vec<_>>>()?;
            Ok(Value::string(parts.join(string(&args[1])?)))
        }
        "parse_int" => string(&args[0])?
            .trim()
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|e| error(format!("parse_int(): {e}"))),
        "parse_float" => {
            let number = string(&args[0])?
                .trim()
                .parse::<f64>()
                .map_err(|e| error(format!("parse_float(): {e}")))?;
            if !number.is_finite() {
                return Err(error("parse_float() requires a finite number"));
            }
            Ok(Value::Float(number))
        }
        "contains" => Ok(Value::Bool(match &args[0] {
            Value::String(s) => s.contains(string(&args[1])?),
            Value::List(items) | Value::Tuple(items) => items.contains(&args[1]),
            Value::Map(items) => items.contains_key(string(&args[1])?),
            v => {
                return Err(error(format!(
                    "contains() not supported for {}",
                    v.type_name()
                )));
            }
        })),
        "read_text" => {
            let path = string(&args[0])?;
            std::fs::read_to_string(path)
                .map(Value::string)
                .map_err(|e| error(format!("read_text({path:?}): {e}")))
        }
        "read_stdin" => {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| error(format!("read_stdin(): {e}")))?;
            Ok(Value::string(text))
        }
        "Some" | "Ok" | "Err" => Ok(Value::variant(name, args.to_vec())),
        "map" | "filter" => {
            let items = list(&args[0])?;
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let result = rt.call_ref(&args[1], std::slice::from_ref(item))?;
                if name == "map" {
                    out.push(result);
                } else if result.is_truthy() {
                    out.push(item.clone());
                }
            }
            Ok(Value::list(out))
        }
        "reduce" => {
            let mut acc = args[1].clone();
            for item in list(&args[0])? {
                acc = rt.call_ref(&args[2], &[acc, item.clone()])?;
            }
            Ok(acc)
        }
        "sum" => {
            let mut total = Value::Int(0);
            for value in list(&args[0])? {
                number(value)?;
                total = binary(&BinaryOp::Add, total, value.clone())?;
            }
            Ok(total)
        }
        "len" => Ok(Value::Int(match &args[0] {
            Value::List(v) | Value::Tuple(v) => v.len(),
            Value::Map(v) => v.len(),
            Value::String(v) => v.chars().count(),
            v => return Err(error(format!("len() not supported for {}", v.type_name()))),
        } as i64)),
        "range" => make_range(&args[0], &args[1], false),
        "head" => Ok(list(&args[0])?.first().cloned().unwrap_or(Value::Nil)),
        "tail" => Ok(Value::list(
            list(&args[0])?.iter().skip(1).cloned().collect(),
        )),
        "push" => {
            let mut out = list(&args[0])?.to_vec();
            out.push(args[1].clone());
            Ok(Value::list(out))
        }
        "concat" => {
            let mut out = list(&args[0])?.to_vec();
            out.extend_from_slice(list(&args[1])?);
            Ok(Value::list(out))
        }
        "to_map" => {
            let mut out = BTreeMap::new();
            for pair in list(&args[0])? {
                match pair {
                    Value::Tuple(v) | Value::List(v) if v.len() == 2 => {
                        out.insert(map_key(&v[0]), v[1].clone());
                    }
                    _ => return Err(error("to_map() expects pairs")),
                }
            }
            Ok(Value::map(out))
        }
        "to_string" => Ok(Value::string(args[0].to_string())),
        "print" | "println" => {
            rt.write(&format!(
                "{}{}",
                args[0],
                if name == "println" { "\n" } else { "" }
            ))?;
            Ok(Value::Nil)
        }
        "abs" => match args[0] {
            Value::Int(i) => i
                .checked_abs()
                .map(Value::Int)
                .ok_or_else(|| error("Integer overflow")),
            Value::Float(f) => Ok(Value::Float(f.abs())),
            _ => Err(error("abs() expects number")),
        },
        "min" | "max" => {
            number(&args[0])?;
            number(&args[1])?;
            let order = compare(&args[0], &args[1])?;
            Ok(
                if (name == "min" && matches!(order, Some(Ordering::Less | Ordering::Equal)))
                    || (name == "max" && matches!(order, Some(Ordering::Greater | Ordering::Equal)))
                {
                    args[0].clone()
                } else {
                    args[1].clone()
                },
            )
        }
        _ => super::scripting::call(rt, name, args),
    }
}

pub fn arity(callee: &Value) -> RyndResult<usize> {
    match callee {
        Value::Builtin { arity, .. }
        | Value::Native { arity, .. }
        | Value::Compiled { arity, .. }
        | Value::Function { arity, .. }
        | Value::Closure { arity, .. } => Ok(*arity),
        v => Err(error(format!("Cannot call {}", v.type_name()))),
    }
}

pub fn check_arity(callee: &Value, found: usize) -> RyndResult<()> {
    let expected = arity(callee)?;
    if expected == found {
        Ok(())
    } else {
        Err(error(format!(
            "Expected {expected} arguments, found {found}"
        )))
    }
}

pub struct NativeRuntime {
    globals: HashMap<String, Value>,
    depth: usize,
}
impl NativeRuntime {
    /// Register Rust functionality for generated Rynd code.
    pub fn register_fn(&mut self, name: &str, arity: usize, func: super::value::NativeFunction) {
        self.globals.insert(
            name.into(),
            Value::Native {
                name: name.into(),
                arity,
                func,
            },
        );
    }
    pub fn register_closure<F>(&mut self, name: &str, arity: usize, func: F)
    where
        F: Fn(&[Value]) -> RyndResult<Value> + 'static,
    {
        self.globals.insert(
            name.into(),
            Value::Compiled {
                name: name.into(),
                arity,
                func: std::rc::Rc::new(move |_, args, _| func(args)),
            },
        );
    }
}
impl Default for NativeRuntime {
    fn default() -> Self {
        Self {
            globals: globals(),
            depth: 0,
        }
    }
}
impl Runtime for NativeRuntime {
    fn call(&mut self, callee: Value, args: Vec<Value>) -> RyndResult<Value> {
        self.call_ref(&callee, &args)
    }
    fn call_ref(&mut self, callee: &Value, args: &[Value]) -> RyndResult<Value> {
        check_arity(callee, args.len())?;
        match callee {
            Value::Compiled { func, .. } => {
                if self.depth >= 256 {
                    return Err(error("Call depth limit exceeded (256)"));
                }
                self.depth += 1;
                let result = func(self, args, callee);
                self.depth -= 1;
                result
            }
            Value::Native { func, .. } => func(args),
            Value::Builtin { name, .. } => call_builtin(self, name, args),
            _ => Err(error("Bytecode function cannot execute in native runtime")),
        }
    }
    fn write(&mut self, text: &str) -> RyndResult<()> {
        std::io::stdout()
            .write_all(text.as_bytes())
            .map_err(|e| RyndError::IoError(e.to_string()))
    }
    fn get_global(&self, name: &str) -> RyndResult<Value> {
        self.globals
            .get(name)
            .cloned()
            .ok_or_else(|| error(format!("Undefined variable '{name}'")))
    }
    fn set_global(&mut self, name: &str, value: Value) {
        self.globals.insert(name.into(), value);
    }
}
