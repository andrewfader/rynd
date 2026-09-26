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
        "sort_by" | "group_by" | "any" | "all" | "find" | "filter_map" | "flat_map"
    ) {
        check_arity(&args[1], 1)?;
    }
    match name {
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
