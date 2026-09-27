//! Eager collection operations with shared VM/native semantics.
use super::{
    runtime::{Runtime, check_arity, compare, error},
    value::Value,
};
use crate::error::RyndResult;
use std::collections::BTreeMap;
use std::convert::TryFrom;

fn list(value: &Value) -> RyndResult<&[Value]> {
    match value {
        Value::List(items) => Ok(items),
        _ => Err(error(format!("Expected list, found {}", value.type_name()))),
    }
}

pub fn call(rt: &mut dyn Runtime, name: &str, args: &[Value]) -> RyndResult<Value> {
    if matches!(name, "keys" | "values" | "entries") {
        let Value::Map(fields) = &args[0] else {
            return Err(error("Expected map"));
        };
        return Ok(Value::list(
            fields
                .iter()
                .map(|(k, v)| match name {
                    "keys" => Value::string(k),
                    "values" => v.clone(),
                    _ => Value::tuple(vec![Value::string(k), v.clone()]),
                })
                .collect(),
        ));
    }
    let items = list(&args[0])?;
    if matches!(
        name,
        "sort_by"
            | "group_by"
            | "any"
            | "all"
            | "find"
            | "filter_map"
            | "flat_map"
            | "each"
            | "partition"
    ) {
        check_arity(&args[1], 1)?;
    }
    match name {
        "chunks" | "windows" => {
            let n = i64::try_from(&args[1])?;
            let n = usize::try_from(n)
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| error("Batch size must be a positive integer"))?;
            let groups: Vec<Value> = if name == "chunks" {
                items.chunks(n).map(|v| Value::list(v.to_vec())).collect()
            } else {
                items.windows(n).map(|v| Value::list(v.to_vec())).collect()
            };
            Ok(Value::list(groups))
        }
        "reverse" => Ok(Value::list(items.iter().rev().cloned().collect())),
        "uniq" => {
            let mut out = Vec::new();
            for item in items {
                if !out.contains(item) {
                    out.push(item.clone());
                }
            }
            Ok(Value::list(out))
        }
        "flatten" => {
            let mut out = Vec::new();
            for item in items {
                out.extend_from_slice(list(item)?);
            }
            Ok(Value::list(out))
        }
        "each" | "partition" => {
            let mut yes = Vec::new();
            let mut no = Vec::new();
            for item in items {
                let result = rt.call_ref(&args[1], std::slice::from_ref(item))?;
                if name == "partition" {
                    if result.is_truthy() {
                        yes.push(item.clone());
                    } else {
                        no.push(item.clone());
                    }
                }
            }
            Ok(if name == "each" {
                args[0].clone()
            } else {
                Value::tuple(vec![Value::list(yes), Value::list(no)])
            })
        }
        "zip_with" => {
            let other = list(&args[1])?;
            check_arity(&args[2], 2)?;
            let mut out = Vec::new();
            for (a, b) in items.iter().zip(other) {
                out.push(rt.call_ref(&args[2], &[a.clone(), b.clone()])?);
            }
            Ok(Value::list(out))
        }
        "scan" => {
            check_arity(&args[2], 2)?;
            let mut accumulator = args[1].clone();
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                accumulator = rt.call_ref(&args[2], &[accumulator, item.clone()])?;
                out.push(accumulator.clone());
            }
            Ok(Value::list(out))
        }
        "take" | "skip" => {
            let Value::Int(n) = args[1] else {
                return Err(error("Count must be a nonnegative integer"));
            };
            let n = usize::try_from(n)
                .map_err(|_| error("Count must be a nonnegative integer"))?
                .min(items.len());
            Ok(Value::list(
                if name == "take" {
                    &items[..n]
                } else {
                    &items[n..]
                }
                .to_vec(),
            ))
        }
        "enumerate" => Ok(Value::list(
            items
                .iter()
                .enumerate()
                .map(|(i, value)| Value::tuple(vec![Value::Int(i as i64), value.clone()]))
                .collect(),
        )),
        "zip" => Ok(Value::list(
            items
                .iter()
                .zip(list(&args[1])?)
                .map(|(a, b)| Value::tuple(vec![a.clone(), b.clone()]))
                .collect(),
        )),
        "any" | "all" | "find" => {
            for item in items {
                let yes = rt
                    .call_ref(&args[1], std::slice::from_ref(item))?
                    .is_truthy();
                if name == "find" && yes {
                    return Ok(Value::variant("Some", vec![item.clone()]));
                }
                if name == "any" && yes {
                    return Ok(Value::Bool(true));
                }
                if name == "all" && !yes {
                    return Ok(Value::Bool(false));
                }
            }
            Ok(if name == "find" {
                Value::variant("None", vec![])
            } else {
                Value::Bool(name == "all")
            })
        }
        "filter_map" | "flat_map" => {
            let mut out = Vec::new();
            for item in items {
                let value = rt.call_ref(&args[1], std::slice::from_ref(item))?;
                if name == "flat_map" {
                    out.extend_from_slice(list(&value)?);
                } else {
                    match value {
                        Value::Variant { name, values } if name == "Some" && values.len() == 1 => {
                            out.push(values[0].clone())
                        }
                        Value::Variant { name, values } if name == "None" && values.is_empty() => {}
                        _ => {
                            return Err(error(
                                "filter_map() callback must return Some(value) or None",
                            ));
                        }
                    }
                }
            }
            Ok(Value::list(out))
        }
        "group_by" => {
            let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
            for item in items {
                let key = rt.call_ref(&args[1], std::slice::from_ref(item))?;
                let Value::String(key) = key else {
                    return Err(error("group_by() callback must return a string key"));
                };
                groups.entry((*key).clone()).or_default().push(item.clone());
            }
            Ok(Value::map(
                groups
                    .into_iter()
                    .map(|(k, v)| (k, Value::list(v)))
                    .collect(),
            ))
        }
        "sort" | "sort_by" => {
            // Evaluate keys exactly once, in input order. Validate the ordering
            // domain before sorting so the comparator is total and infallible.
            let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(items.len());
            for item in items {
                let key = if name == "sort" {
                    item.clone()
                } else {
                    rt.call_ref(&args[1], std::slice::from_ref(item))?
                };
                if compare(&key, &key)?.is_none() {
                    return Err(error("Cannot sort NaN"));
                }
                if let Some((first, _)) = keyed.first() {
                    compare(first, &key)?;
                }
                keyed.push((key, item.clone()));
            }
            keyed.sort_by(|(a, _), (b, _)| compare(a, b).unwrap().unwrap());
            Ok(Value::list(keyed.into_iter().map(|(_, v)| v).collect()))
        }
        _ => Err(error(format!("Unknown collection builtin '{name}'"))),
    }
}
