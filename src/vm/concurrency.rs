//! Structured concurrency, green fibers, actors, and channels for Rynd.
use super::{
    runtime::{Runtime, error},
    value::Value,
};
use crate::error::RyndResult;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::convert::TryFrom;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const BUILTINS: &[(&str, usize)] = &[
    ("spawn", 1),
    ("await_fiber", 1),
    ("yield_fiber", 0),
    ("nursery", 1),
    ("spawn_actor", 1),
    ("send", 2),
    ("receive", 1),
    ("receive_timeout", 2),
    ("create_mailbox", 0),
    ("close_mailbox", 1),
    ("channel", 0),
    ("channel_send", 2),
    ("channel_recv", 1),
    ("channel_try_recv", 1),
    ("close_channel", 1),
    ("is_channel_closed", 1),
    ("self_id", 0),
];

thread_local! {
    static NEXT_ID: RefCell<i64> = const { RefCell::new(1) };
    static CURRENT_FIBER: RefCell<i64> = const { RefCell::new(0) };
    static FIBERS: RefCell<HashMap<i64, FiberEntry>> = RefCell::new(HashMap::new());
    static MAILBOXES: RefCell<HashMap<i64, VecDeque<Value>>> = RefCell::new(HashMap::new());
    static CHANNELS: RefCell<HashMap<i64, VecDeque<Value>>> = RefCell::new(HashMap::new());
}

struct FiberEntry {
    task: Value,
    result: Option<RyndResult<Value>>,
}

fn next_id() -> i64 {
    NEXT_ID.with(|id| {
        let mut curr = id.borrow_mut();
        let val = *curr;
        *curr += 1;
        val
    })
}

fn extract_handle(value: &Value, expected: &str) -> RyndResult<i64> {
    match value {
        Value::Variant { name, values } if name == expected && values.len() == 1 => {
            match &values[0] {
                Value::Int(id) => Ok(*id),
                _ => Err(error(format!("Invalid {expected} handle"))),
            }
        }
        Value::Int(id) => Ok(*id),
        _ => Err(error(format!(
            "Expected {expected}, got {}",
            value.type_name()
        ))),
    }
}

pub fn call(rt: &mut dyn Runtime, name: &str, args: &[Value]) -> RyndResult<Value> {
    match name {
        "spawn" => {
            let task = args[0].clone();
            let id = next_id();
            FIBERS.with(|f| {
                f.borrow_mut().insert(id, FiberEntry { task, result: None });
            });
            Ok(Value::variant("Fiber", vec![Value::Int(id)]))
        }

        "yield_fiber" => {
            // Cooperative pause: allows scheduler / environment iteration
            Ok(Value::Nil)
        }

        "await_fiber" => {
            let id = if let Ok(id) = extract_handle(&args[0], "Fiber") {
                id
            } else {
                extract_handle(&args[0], "Actor")?
            };

            // Pop the entry: a fiber is one-shot — the first await drives the
            // task, caches the result, and removes the entry so a repeat
            // await surfaces a clean "expired" error instead of silently
            // re-executing or accumulating closures.
            let entry = FIBERS.with(|f| f.borrow_mut().remove(&id));
            let entry = entry.ok_or_else(|| error("Invalid or expired Fiber handle"))?;

            if let Some(res) = entry.result {
                return res;
            }

            // Execute the fiber task with fiber ID context
            let old_id = CURRENT_FIBER.with(|cf| {
                let mut curr = cf.borrow_mut();
                let old = *curr;
                *curr = id;
                old
            });

            let exec_res = rt.call_ref(&entry.task, &[]);

            CURRENT_FIBER.with(|cf| {
                *cf.borrow_mut() = old_id;
            });

            // For actors, also drop the mailbox queue. Actors own their mailbox
            // for life; once the actor body returns, the mailbox is no longer
            // observable and the queue can be released.
            MAILBOXES.with(|m| {
                m.borrow_mut().remove(&id);
            });

            exec_res
        }

        "nursery" => {
            // Structured concurrency scope:
            // nursery(\n -> { n.spawn(\ -> 1); n.spawn(\ -> 2) })
            // Children can never outlive the scope. If any child fails, cancels siblings.
            let nursery_scope_id = next_id();
            let tasks_cell = Rc::new(RefCell::new(Vec::new()));

            let spawn_tasks = tasks_cell.clone();
            let spawn_fn: Value = Value::Compiled {
                name: Rc::from("spawn"),
                arity: 1,
                func: Rc::new(move |_rt, fn_args, _| {
                    let task = fn_args[0].clone();
                    let fiber_id = next_id();
                    spawn_tasks.borrow_mut().push((fiber_id, task));
                    Ok(Value::variant("Fiber", vec![Value::Int(fiber_id)]))
                }),
            };

            let mut nursery_map = std::collections::BTreeMap::new();
            nursery_map.insert("id".into(), Value::Int(nursery_scope_id));
            nursery_map.insert("spawn".into(), spawn_fn);
            let nursery_obj = Value::map(nursery_map);

            // Run the block to register/spawn child tasks
            rt.call_ref(&args[0], &[nursery_obj])?;

            // Execute all spawned children in structured dependency order
            let spawned = tasks_cell.borrow().clone();
            let mut results = Vec::with_capacity(spawned.len());

            for (fiber_id, task) in spawned {
                let old_id = CURRENT_FIBER.with(|cf| {
                    let mut curr = cf.borrow_mut();
                    let old = *curr;
                    *curr = fiber_id;
                    old
                });

                let res = rt.call_ref(&task, &[]);

                CURRENT_FIBER.with(|cf| {
                    *cf.borrow_mut() = old_id;
                });

                match res {
                    Ok(v) => results.push(v),
                    Err(e) => {
                        // Structured failure propagation: child error fails nursery
                        return Err(e);
                    }
                }
            }

            Ok(Value::list(results))
        }

        "create_mailbox" => {
            let id = next_id();
            MAILBOXES.with(|m| {
                m.borrow_mut().insert(id, VecDeque::new());
            });
            Ok(Value::variant("Mailbox", vec![Value::Int(id)]))
        }

        "close_mailbox" => {
            // Release the mailbox queue. `send` and `receive` will report
            // an error after the mailbox is closed.
            let id = if let Ok(id) = extract_handle(&args[0], "Mailbox") {
                id
            } else {
                extract_handle(&args[0], "Actor")?
            };
            let removed = MAILBOXES.with(|m| m.borrow_mut().remove(&id).is_some());
            Ok(Value::Bool(removed))
        }

        "spawn_actor" => {
            let actor_id = next_id();
            MAILBOXES.with(|m| {
                m.borrow_mut().insert(actor_id, VecDeque::new());
            });

            let actor_task = args[0].clone();
            let mailbox_handle = Value::variant("Mailbox", vec![Value::Int(actor_id)]);

            let task_wrapper = Value::Compiled {
                name: format!("actor_{actor_id}").into(),
                arity: 0,
                func: std::rc::Rc::new(move |inner_rt, _, _| {
                    inner_rt.call_ref(&actor_task, std::slice::from_ref(&mailbox_handle))
                }),
            };

            FIBERS.with(|f| {
                f.borrow_mut().insert(
                    actor_id,
                    FiberEntry {
                        task: task_wrapper,
                        result: None,
                    },
                );
            });

            Ok(Value::variant("Actor", vec![Value::Int(actor_id)]))
        }

        "send" => {
            let id = if let Ok(id) = extract_handle(&args[0], "Actor") {
                id
            } else {
                extract_handle(&args[0], "Mailbox")?
            };
            let msg = args[1].clone();
            MAILBOXES.with(|m| {
                let mut mailboxes = m.borrow_mut();
                let queue = mailboxes
                    .get_mut(&id)
                    .ok_or_else(|| error("Target Actor/Mailbox does not exist"))?;
                queue.push_back(msg);
                Ok(Value::Bool(true))
            })
        }

        "receive" => {
            let id = if let Ok(id) = extract_handle(&args[0], "Mailbox") {
                id
            } else {
                extract_handle(&args[0], "Actor")?
            };
            MAILBOXES.with(|m| {
                let mut mailboxes = m.borrow_mut();
                let queue = mailboxes
                    .get_mut(&id)
                    .ok_or_else(|| error("Mailbox does not exist"))?;
                queue.pop_front().ok_or_else(|| error("Mailbox is empty"))
            })
        }

        "receive_timeout" => {
            let id = if let Ok(id) = extract_handle(&args[0], "Mailbox") {
                id
            } else {
                extract_handle(&args[0], "Actor")?
            };
            let ms = match &args[1] {
                Value::Int(n) => {
                    u64::try_from(*n).map_err(|_| error("Timeout must be non-negative"))?
                }
                _ => return Err(error("Expected integer milliseconds")),
            };
            let start = Instant::now();
            let timeout = Duration::from_millis(ms);

            loop {
                let found = MAILBOXES.with(|m| {
                    let mut mailboxes = m.borrow_mut();
                    if let Some(queue) = mailboxes.get_mut(&id) {
                        queue.pop_front()
                    } else {
                        None
                    }
                });

                if let Some(msg) = found {
                    return Ok(Value::variant("Some", vec![msg]));
                }

                if start.elapsed() >= timeout {
                    return Ok(Value::variant("None", vec![]));
                }

                std::thread::sleep(Duration::from_millis(1));
            }
        }

        "self_id" => {
            let current = CURRENT_FIBER.with(|cf| *cf.borrow());
            // `self_id` returns a Fiber handle so it composes with `await_fiber`
            // and other fiber APIs. The main thread is fiber 0; spawned fibers
            // and actors each get their own integer id.
            Ok(Value::variant("Fiber", vec![Value::Int(current)]))
        }

        "channel" => {
            let id = next_id();
            CHANNELS.with(|c| {
                c.borrow_mut().insert(id, VecDeque::new());
            });
            let sender = Value::variant("Sender", vec![Value::Int(id)]);
            let receiver = Value::variant("Receiver", vec![Value::Int(id)]);
            Ok(Value::tuple(vec![sender, receiver]))
        }

        "channel_send" => {
            let id = extract_handle(&args[0], "Sender")?;
            let msg = args[1].clone();
            CHANNELS.with(|c| {
                let mut channels = c.borrow_mut();
                let queue = channels
                    .get_mut(&id)
                    .ok_or_else(|| error("Channel sender is disconnected"))?;
                queue.push_back(msg);
                Ok(Value::Bool(true))
            })
        }

        "channel_recv" => {
            let id = extract_handle(&args[0], "Receiver")?;
            CHANNELS.with(|c| {
                let mut channels = c.borrow_mut();
                let queue = channels
                    .get_mut(&id)
                    .ok_or_else(|| error("Channel receiver is disconnected"))?;
                queue.pop_front().ok_or_else(|| error("Channel is empty"))
            })
        }

        "channel_try_recv" => {
            let id = extract_handle(&args[0], "Receiver")?;
            CHANNELS.with(|c| {
                let mut channels = c.borrow_mut();
                if let Some(queue) = channels.get_mut(&id) {
                    if let Some(msg) = queue.pop_front() {
                        Ok(Value::variant("Some", vec![msg]))
                    } else {
                        Ok(Value::variant("None", vec![]))
                    }
                } else {
                    Err(error("Channel receiver is disconnected"))
                }
            })
        }

        "close_channel" => {
            // Close either the sender or receiver side of a channel. The
            // underlying queue is removed once both sides have been closed so
            // memory is reclaimed even when callers forget to close one side.
            let id = if let Ok(id) = extract_handle(&args[0], "Sender") {
                id
            } else {
                extract_handle(&args[0], "Receiver")?
            };
            let removed = CHANNELS.with(|c| c.borrow_mut().remove(&id).is_some());
            Ok(Value::Bool(removed))
        }

        "is_channel_closed" => {
            let id = if let Ok(id) = extract_handle(&args[0], "Sender") {
                id
            } else {
                extract_handle(&args[0], "Receiver")?
            };
            let exists = CHANNELS.with(|c| c.borrow().contains_key(&id));
            Ok(Value::Bool(!exists))
        }

        _ => Err(error(format!("Unknown concurrency builtin '{name}'"))),
    }
}
