//! An embedded document store: named collections of JSON documents keyed by
//! integer or string ids, kept in memory and persisted to an append-only
//! JSON Lines log. Opening replays the log; a log that has grown well past
//! its live data is compacted (rewritten and atomically renamed).
//!
//! Every write is appended before the call returns, so a crashed process
//! loses nothing it acknowledged; a torn final line from a crash is skipped.
//! Fibers switch only at blocking points, so a read-check-write sequence in
//! one fiber is never interleaved with another fiber's writes.
use super::{
    concurrency::GroupId,
    runtime::{Runtime, error},
    value::Value,
};
use crate::error::RyndResult;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering as AtomicOrdering};
use std::sync::{LazyLock, Mutex};

pub const BUILTINS: &[(&str, usize)] = &[
    ("store_open", 1),
    ("store_close", 1),
    ("store_put", 4),
    ("store_get", 3),
    ("store_delete", 3),
    ("store_all", 2),
    ("store_find", 3),
    ("store_count", 2),
    ("store_next_id", 2),
];

/// Document ids: integers sort numerically and before strings.
#[derive(Clone, PartialEq, Eq)]
enum Key {
    Int(i64),
    Str(String),
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Key::Int(a), Key::Int(b)) => a.cmp(b),
            (Key::Str(a), Key::Str(b)) => a.cmp(b),
            (Key::Int(_), Key::Str(_)) => Ordering::Less,
            (Key::Str(_), Key::Int(_)) => Ordering::Greater,
        }
    }
}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Key {
    fn from_value(value: &Value) -> RyndResult<Key> {
        match value {
            Value::Int(n) => Ok(Key::Int(*n)),
            Value::String(s) => Ok(Key::Str(s.as_str().into())),
            other => Err(error(format!(
                "Document ids must be integers or strings, found {}",
                other.type_name()
            ))),
        }
    }
    fn to_value(&self) -> Value {
        match self {
            Key::Int(n) => Value::Int(*n),
            Key::Str(s) => Value::string(s.clone()),
        }
    }
}

type Collection = BTreeMap<Key, Value>;

struct Store {
    owner: GroupId,
    path: Option<PathBuf>,
    log: Option<File>,
    collections: BTreeMap<String, Collection>,
    sequences: BTreeMap<String, i64>,
    /// Log lines written since the last compaction, for the rewrite heuristic.
    entries: usize,
}

/// SAFETY: a store's values are only touched under `STORES`' mutex by threads
/// of its owning group, which run script code one at a time (see
/// `concurrency`); `store()` rejects every other thread.
struct Owned(Store);
unsafe impl Send for Owned {}

static NEXT_STORE: AtomicI64 = AtomicI64::new(1);
static STORES: LazyLock<Mutex<HashMap<i64, Owned>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn io_error(path: &std::path::Path, e: std::io::Error) -> crate::error::RyndError {
    error(format!("store {}: {e}", path.display()))
}

fn record(op: &str, fields: Vec<Value>) -> RyndResult<String> {
    let mut map = BTreeMap::new();
    map.insert(op.to_string(), Value::list(fields));
    Ok(super::json::stringify(&Value::map(map))? + "\n")
}

impl Store {
    fn live(&self) -> usize {
        self.collections.values().map(BTreeMap::len).sum::<usize>() + self.sequences.len()
    }

    fn apply(&mut self, line: &str) -> Result<(), String> {
        let Ok(Value::Map(entry)) = super::json::parse(line) else {
            return Err("not a JSON object".into());
        };
        let (op, fields) = entry.iter().next().ok_or("empty entry")?;
        let Value::List(fields) = fields else {
            return Err("fields must be a list".into());
        };
        let name = |i: usize| match fields.get(i) {
            Some(Value::String(s)) => Ok(s.as_str().to_string()),
            _ => Err(format!("{op}: field {i} must be a collection name")),
        };
        let key = |i: usize| {
            fields
                .get(i)
                .ok_or_else(|| format!("{op}: missing id"))
                .and_then(|v| Key::from_value(v).map_err(|e| e.to_string()))
        };
        match (op.as_str(), fields.len()) {
            ("put", 3) => {
                self.collections
                    .entry(name(0)?)
                    .or_default()
                    .insert(key(1)?, fields[2].clone());
            }
            ("del", 2) => {
                if let Some(collection) = self.collections.get_mut(&name(0)?) {
                    collection.remove(&key(1)?);
                }
            }
            ("seq", 2) => {
                let Value::Int(n) = fields[1] else {
                    return Err("seq: counter must be an integer".into());
                };
                self.sequences.insert(name(0)?, n);
            }
            _ => return Err(format!("unknown entry {op:?}")),
        }
        Ok(())
    }

    fn open(path: Option<PathBuf>) -> RyndResult<Store> {
        let mut store = Store {
            owner: GroupId::current(),
            path: path.clone(),
            log: None,
            collections: BTreeMap::new(),
            sequences: BTreeMap::new(),
            entries: 0,
        };
        let Some(path) = path else {
            return Ok(store);
        };
        if path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
            let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
            for (i, line) in lines.iter().enumerate() {
                if let Err(why) = store.apply(line) {
                    // A crash can tear only the last append; anything earlier is corruption.
                    if i + 1 == lines.len() && !text.ends_with('\n') {
                        break;
                    }
                    return Err(error(format!(
                        "store {}: line {}: {why}",
                        path.display(),
                        i + 1
                    )));
                }
                store.entries += 1;
            }
            if store.entries > 2 * store.live() + 64 || !text.is_empty() && !text.ends_with('\n') {
                store.compact()?;
            }
        }
        if store.log.is_none() {
            store.log = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .map_err(|e| io_error(&path, e))?,
            );
        }
        Ok(store)
    }

    /// Rewrite the log as one entry per live document, then swap it in.
    fn compact(&mut self) -> RyndResult<()> {
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        let mut text = String::new();
        for (name, n) in &self.sequences {
            text.push_str(&record(
                "seq",
                vec![Value::string(name.clone()), Value::Int(*n)],
            )?);
        }
        for (name, collection) in &self.collections {
            for (key, doc) in collection {
                text.push_str(&record(
                    "put",
                    vec![Value::string(name.clone()), key.to_value(), doc.clone()],
                )?);
            }
        }
        let tmp = path.with_extension("compacting");
        std::fs::write(&tmp, &text).map_err(|e| io_error(&tmp, e))?;
        File::open(&tmp)
            .and_then(|f| f.sync_all())
            .map_err(|e| io_error(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| io_error(&path, e))?;
        self.log = Some(
            OpenOptions::new()
                .append(true)
                .open(&path)
                .map_err(|e| io_error(&path, e))?,
        );
        self.entries = self.live();
        Ok(())
    }

    fn append(&mut self, line: String) -> RyndResult<()> {
        if let (Some(log), Some(path)) = (&mut self.log, &self.path) {
            log.write_all(line.as_bytes())
                .map_err(|e| io_error(path, e))?;
            self.entries += 1;
            if self.entries > 4 * self.live() + 1024 {
                self.compact()?;
            }
        }
        Ok(())
    }
}

fn handle(value: &Value) -> RyndResult<i64> {
    match value {
        Value::Variant { name, values } if name == "Store" && values.len() == 1 => {
            match values[0] {
                Value::Int(id) => Ok(id),
                _ => Err(error("Invalid Store handle")),
            }
        }
        other => Err(error(format!(
            "Expected Store, found {}",
            other.type_name()
        ))),
    }
}

fn with_store<T>(value: &Value, f: impl FnOnce(&mut Store) -> RyndResult<T>) -> RyndResult<T> {
    let id = handle(value)?;
    let mut stores = STORES.lock().unwrap_or_else(|p| p.into_inner());
    let Owned(store) = stores
        .get_mut(&id)
        .ok_or_else(|| error("Store handle is closed or invalid"))?;
    if !store.owner.is_current() {
        return Err(error(
            "This store belongs to another program thread; open it where it is used",
        ));
    }
    f(store)
}

fn name(value: &Value) -> RyndResult<String> {
    match value {
        Value::String(s) if !s.is_empty() => Ok(s.as_str().to_string()),
        _ => Err(error("Collection names must be non-empty strings")),
    }
}

fn matches(doc: &Value, filter: &BTreeMap<String, Value>) -> bool {
    let Value::Map(fields) = doc else {
        return filter.is_empty();
    };
    filter.iter().all(|(k, v)| fields.get(k) == Some(v))
}

pub fn call(_rt: &mut dyn Runtime, builtin: &str, args: &[Value]) -> RyndResult<Value> {
    match builtin {
        "store_open" => {
            let path = <&str>::try_from(&args[0])?;
            let path = (!path.is_empty() && path != ":memory:").then(|| PathBuf::from(path));
            let store = Store::open(path)?;
            let id = NEXT_STORE.fetch_add(1, AtomicOrdering::Relaxed);
            STORES
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(id, Owned(store));
            Ok(Value::variant("Store", vec![Value::Int(id)]))
        }
        "store_close" => {
            with_store(&args[0], |_| Ok(()))?;
            let id = handle(&args[0])?;
            let removed = STORES.lock().unwrap_or_else(|p| p.into_inner()).remove(&id);
            if let Some(Owned(Store {
                log: Some(log),
                path: Some(path),
                ..
            })) = removed
            {
                log.sync_all().map_err(|e| io_error(&path, e))?;
            }
            Ok(Value::Nil)
        }
        "store_put" => {
            let collection = name(&args[1])?;
            let key = Key::from_value(&args[2])?;
            let doc = args[3].clone();
            with_store(&args[0], |store| {
                let line = record(
                    "put",
                    vec![
                        Value::string(collection.clone()),
                        key.to_value(),
                        doc.clone(),
                    ],
                )?;
                store.append(line)?;
                store
                    .collections
                    .entry(collection)
                    .or_default()
                    .insert(key, doc.clone());
                Ok(doc)
            })
        }
        "store_get" => {
            let collection = name(&args[1])?;
            let key = Key::from_value(&args[2])?;
            with_store(&args[0], |store| {
                Ok(store
                    .collections
                    .get(&collection)
                    .and_then(|c| c.get(&key))
                    .cloned()
                    .unwrap_or(Value::Nil))
            })
        }
        "store_delete" => {
            let collection = name(&args[1])?;
            let key = Key::from_value(&args[2])?;
            with_store(&args[0], |store| {
                let present = store
                    .collections
                    .get(&collection)
                    .is_some_and(|c| c.contains_key(&key));
                if present {
                    store.append(record(
                        "del",
                        vec![Value::string(collection.clone()), key.to_value()],
                    )?)?;
                    if let Some(c) = store.collections.get_mut(&collection) {
                        c.remove(&key);
                    }
                }
                Ok(Value::Bool(present))
            })
        }
        "store_all" | "store_count" => {
            let collection = name(&args[1])?;
            with_store(&args[0], |store| {
                let docs = store.collections.get(&collection);
                Ok(if builtin == "store_count" {
                    Value::Int(docs.map_or(0, BTreeMap::len) as i64)
                } else {
                    Value::list(
                        docs.map(|c| c.values().cloned().collect())
                            .unwrap_or_default(),
                    )
                })
            })
        }
        "store_find" => {
            let collection = name(&args[1])?;
            let Value::Map(filter) = &args[2] else {
                return Err(error("store_find() expects a map of field values"));
            };
            with_store(&args[0], |store| {
                Ok(Value::list(
                    store
                        .collections
                        .get(&collection)
                        .map(|c| {
                            c.values()
                                .filter(|doc| matches(doc, filter))
                                .cloned()
                                .collect()
                        })
                        .unwrap_or_default(),
                ))
            })
        }
        "store_next_id" => {
            let collection = name(&args[1])?;
            with_store(&args[0], |store| {
                let next = store
                    .sequences
                    .get(&collection)
                    .copied()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| error("Store id sequence overflow"))?;
                store.append(record(
                    "seq",
                    vec![Value::string(collection.clone()), Value::Int(next)],
                )?)?;
                store.sequences.insert(collection, next);
                Ok(Value::Int(next))
            })
        }
        _ => Err(error(format!("Unknown store builtin '{builtin}'"))),
    }
}
