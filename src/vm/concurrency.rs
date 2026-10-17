//! Fibers, structured nurseries, actors, and channels for Rynd.
//!
//! Every fiber runs on its own OS thread with its own stack, so a fiber can
//! suspend anywhere: inside a `map` callback, deep in recursion, or in a
//! socket read. A baton serializes script execution within a scheduler group
//! (the thread that started the program plus every fiber it spawned): exactly
//! one fiber runs Rynd code at a time, and the baton moves at scheduling
//! points — `yield_fiber`, `await_fiber`, `receive`, `channel_recv`, nursery
//! joins, and blocking I/O. Blocking I/O releases the baton for its whole
//! duration, so a server fiber waiting on a slow client never stalls the rest.
use super::{
    runtime::{Runtime, error},
    value::Value,
};
use crate::error::RyndResult;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::convert::TryFrom;
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
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

/// Fiber threads reserve this much address space for their stack; pages are
/// committed only as recursion actually touches them.
const FIBER_STACK_BYTES: usize = 256 * 1024 * 1024;

static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn next_id() -> i64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Moves script values between the threads of one scheduler group.
///
/// Values use `Rc`, which is not thread-safe. They are sound to share here
/// because a group runs script code on one thread at a time: a value is only
/// touched by the baton holder, and the baton changes hands under the group
/// mutex, which orders every access before the next holder's.
struct Baton<T>(T);
unsafe impl<T> Send for Baton<T> {}

struct Slot {
    result: Option<RyndResult<Value>>,
    cancelled: bool,
    /// Detached fibers (server connections) drop their result on completion.
    detached: bool,
}

#[derive(Default)]
struct State {
    running: Option<i64>,
    ready: VecDeque<i64>,
    /// Fibers parked on a script condition; any state change requeues them.
    waiting: Vec<i64>,
    /// Fibers that will come back without help: blocking I/O and timed waits.
    away: usize,
    wakers: HashMap<i64, Arc<Condvar>>,
    fibers: HashMap<i64, Slot>,
    mailboxes: HashMap<i64, VecDeque<Value>>,
    channels: HashMap<i64, VecDeque<Value>>,
}

struct Group {
    state: Mutex<State>,
}
// SAFETY: see `Baton`; the values inside `State` are only touched under the
// mutex, by threads of this group.
unsafe impl Send for Group {}
unsafe impl Sync for Group {}

thread_local! {
    static CURRENT: RefCell<Option<(Arc<Group>, i64)>> = const { RefCell::new(None) };
}

/// The calling thread's group and fiber id. A thread that never ran inside a
/// fiber starts a group of its own as fiber 0.
fn current() -> (Arc<Group>, i64) {
    CURRENT.with(|cell| {
        cell.borrow_mut()
            .get_or_insert_with(|| {
                let mut state = State {
                    running: Some(0),
                    ..State::default()
                };
                state.wakers.insert(0, Arc::new(Condvar::new()));
                (
                    Arc::new(Group {
                        state: Mutex::new(state),
                    }),
                    0,
                )
            })
            .clone()
    })
}

/// Identifies the calling thread's scheduler group. Resources that hold
/// script values (stores) belong to one group and refuse the others, whose
/// threads run in parallel with it.
#[derive(Clone)]
pub struct GroupId(Arc<Group>);

impl GroupId {
    pub fn current() -> Self {
        GroupId(current().0)
    }
    pub fn is_current(&self) -> bool {
        Arc::ptr_eq(&self.0, &current().0)
    }
}

fn lock(group: &Group) -> MutexGuard<'_, State> {
    group.state.lock().unwrap_or_else(|p| p.into_inner())
}

impl State {
    /// Give the baton to the next ready fiber, or to nobody.
    fn pass(&mut self) {
        self.running = self.ready.pop_front();
        if let Some(waker) = self.running.and_then(|next| self.wakers.get(&next)) {
            waker.notify_one();
        }
    }
    /// Requeue every parked fiber so it re-checks its condition.
    fn wake_waiting(&mut self) {
        let parked = std::mem::take(&mut self.waiting);
        for id in parked {
            self.enqueue(id);
        }
    }
    fn enqueue(&mut self, id: i64) {
        if self.running.is_none() {
            self.running = Some(id);
            if let Some(waker) = self.wakers.get(&id) {
                waker.notify_one();
            }
        } else if self.running != Some(id) && !self.ready.contains(&id) {
            self.ready.push_back(id);
        }
    }
    fn cancelled(&self, id: i64) -> bool {
        self.fibers.get(&id).is_some_and(|slot| slot.cancelled)
    }
}

fn waker(state: &State, id: i64) -> Arc<Condvar> {
    state.wakers.get(&id).cloned().expect("fiber waker")
}

/// Block until `id` holds the baton.
fn hold<'a>(mut state: MutexGuard<'a, State>, id: i64) -> MutexGuard<'a, State> {
    let waker = waker(&state, id);
    while state.running != Some(id) {
        state = waker.wait(state).unwrap_or_else(|p| p.into_inner());
    }
    state
}

fn cancelled_error() -> crate::error::RyndError {
    error("Fiber cancelled")
}

/// Run `work` without the baton, so other fibers run while this one blocks
/// on the outside world (sockets, sleeps, processes, stdin).
pub fn blocking<T>(work: impl FnOnce() -> T) -> T {
    let (group, me) = current();
    {
        let mut state = lock(&group);
        if state.ready.is_empty() && state.fibers.is_empty() {
            // Nobody else to run: skip the handoff entirely.
            drop(state);
            return work();
        }
        state.away += 1;
        state.pass();
    }
    let out = work();
    let mut state = lock(&group);
    state.away -= 1;
    state.enqueue(me);
    drop(hold(state, me));
    out
}

/// Sleep without holding the baton; cancellation ends the sleep early.
pub fn sleep(duration: Duration) -> RyndResult<()> {
    let deadline = Instant::now() + duration;
    let woke = park(|_| false, Some(deadline), "sleep")?;
    debug_assert!(!woke);
    Ok(())
}

/// Park until `ready(state)` holds, the deadline passes (`Ok(false)`), or the
/// fiber is cancelled. Parking with nothing left that could wake us is a
/// deadlock and returns an error instead of hanging.
fn park(
    ready: impl Fn(&mut State) -> bool,
    deadline: Option<Instant>,
    what: &str,
) -> RyndResult<bool> {
    let (group, me) = current();
    let mut state = lock(&group);
    loop {
        if state.cancelled(me) {
            return Err(cancelled_error());
        }
        if ready(&mut state) {
            return Ok(true);
        }
        let now = Instant::now();
        if deadline.is_some_and(|d| now >= d) {
            return Ok(false);
        }
        if deadline.is_none() && state.ready.is_empty() && state.away == 0 {
            return Err(error(format!(
                "Deadlock: {what} would wait forever (no other fiber can run)"
            )));
        }
        let waker = waker(&state, me);
        match deadline {
            None => {
                state.waiting.push(me);
                state.pass();
                state = hold(state, me);
            }
            Some(deadline) => {
                state.away += 1;
                state.waiting.push(me);
                state.pass();
                while state.running != Some(me) {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        // Timed out: queue for the baton like any woken fiber.
                        state.waiting.retain(|id| *id != me);
                        state.enqueue(me);
                        state = hold(state, me);
                        break;
                    }
                    state = waker
                        .wait_timeout(state, left)
                        .unwrap_or_else(|p| p.into_inner())
                        .0;
                }
                state.away -= 1;
            }
        }
    }
}

/// Start `task` on a new fiber. The caller keeps the baton; the fiber runs at
/// the caller's next scheduling point.
pub fn spawn_fiber(rt: &dyn Runtime, task: Value, detached: bool) -> RyndResult<i64> {
    spawn_with_id(rt, task, detached, next_id())
}

fn spawn_with_id(rt: &dyn Runtime, task: Value, detached: bool, id: i64) -> RyndResult<i64> {
    let child = rt.fork()?;
    let (group, _) = current();
    {
        let mut state = lock(&group);
        state.wakers.insert(id, Arc::new(Condvar::new()));
        state.fibers.insert(
            id,
            Slot {
                result: None,
                cancelled: false,
                detached,
            },
        );
        state.ready.push_back(id);
    }
    let payload = Baton((child, task));
    let thread_group = group.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("rynd-fiber-{id}"))
        .stack_size(FIBER_STACK_BYTES)
        .spawn(move || run_fiber(thread_group, id, payload));
    if let Err(e) = spawned {
        let mut state = lock(&group);
        state.ready.retain(|f| *f != id);
        state.fibers.remove(&id);
        state.wakers.remove(&id);
        return Err(error(format!("spawn(): could not start fiber thread: {e}")));
    }
    Ok(id)
}

fn run_fiber(group: Arc<Group>, id: i64, payload: Baton<(Box<dyn Runtime>, Value)>) {
    CURRENT.with(|cell| *cell.borrow_mut() = Some((group.clone(), id)));
    super::safety::claim_stack(FIBER_STACK_BYTES);
    let state = hold(lock(&group), id);
    let cancelled = state.cancelled(id);
    drop(state);
    let Baton((mut rt, task)) = payload;
    let result = if cancelled {
        Err(cancelled_error())
    } else {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.call_ref(&task, &[])))
            .unwrap_or_else(|panic| {
                let message = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".into());
                Err(error(format!("Fiber panicked: {message}")))
            })
    };
    // Script values die while this fiber still holds the baton.
    drop(task);
    drop(rt);
    let mut state = lock(&group);
    state.mailboxes.remove(&id);
    match state.fibers.get_mut(&id) {
        Some(slot) if !slot.detached => slot.result = Some(result),
        _ => {
            let cancelled = matches!(state.fibers.get(&id), Some(slot) if slot.cancelled);
            if let (Err(e), false) = (&result, cancelled) {
                eprintln!("rynd: fiber {id} failed: {e}");
            }
            state.fibers.remove(&id);
            drop(result);
        }
    }
    state.wakers.remove(&id);
    state.wake_waiting();
    state.pass();
}

fn cancel(state: &mut State, id: i64) {
    if let Some(slot) = state
        .fibers
        .get_mut(&id)
        .filter(|slot| slot.result.is_none())
    {
        slot.cancelled = true;
        if let Some(pos) = state.waiting.iter().position(|f| *f == id) {
            state.waiting.swap_remove(pos);
            state.enqueue(id);
        }
    }
}

fn await_id(id: i64) -> RyndResult<Value> {
    let (group, me) = current();
    if id == me {
        return Err(error("A fiber cannot await itself"));
    }
    if !lock(&group).fibers.contains_key(&id) {
        return Err(error("Invalid or expired Fiber handle"));
    }
    park(
        |state| {
            state
                .fibers
                .get(&id)
                .is_none_or(|slot| slot.result.is_some())
        },
        None,
        "await_fiber",
    )?;
    let slot = lock(&group).fibers.remove(&id);
    slot.and_then(|slot| slot.result)
        .unwrap_or_else(|| Err(error("Invalid or expired Fiber handle")))
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

fn handle_of(value: &Value, first: &str, second: &str) -> RyndResult<i64> {
    extract_handle(value, first).or_else(|_| extract_handle(value, second))
}

fn nursery(rt: &mut dyn Runtime, block: &Value) -> RyndResult<Value> {
    let children = Rc::new(RefCell::new(Vec::new()));
    let spawned = children.clone();
    let spawn_fn = Value::Compiled {
        name: Rc::from("spawn"),
        arity: 1,
        func: Rc::new(move |rt, args, _| {
            let id = spawn_fiber(rt, args[0].clone(), false)?;
            spawned.borrow_mut().push(id);
            Ok(Value::variant("Fiber", vec![Value::Int(id)]))
        }),
    };
    let mut scope = BTreeMap::new();
    scope.insert("id".into(), Value::Int(next_id()));
    scope.insert("spawn".into(), spawn_fn);
    let body = rt.call_ref(block, &[Value::map(scope)]);
    let ids = children.borrow().clone();
    let (group, _) = current();
    let failed = |state: &State| {
        ids.iter().any(|id| {
            matches!(
                state.fibers.get(id),
                Some(Slot {
                    result: Some(Err(_)),
                    ..
                })
            )
        })
    };
    let done = |state: &State| {
        ids.iter().all(|id| {
            state
                .fibers
                .get(id)
                .is_none_or(|slot| slot.result.is_some())
        })
    };
    // Children never outlive the scope: wait for all of them, canceling the
    // rest as soon as the body or any child fails.
    let mut joined = if body.is_ok() {
        park(|state| done(state) || failed(state), None, "nursery")
    } else {
        Ok(true)
    };
    let mut state = lock(&group);
    if body.is_err() || joined.is_err() || failed(&state) {
        for id in &ids {
            cancel(&mut state, *id);
        }
        drop(state);
        let settled = park(|state| done(state), None, "nursery cancellation");
        joined = joined.and(settled);
        state = lock(&group);
    }
    let mut results = Vec::with_capacity(ids.len());
    let mut first_error = None;
    for id in &ids {
        match state.fibers.remove(id).and_then(|slot| slot.result) {
            Some(Ok(value)) => results.push(value),
            Some(Err(e)) => {
                let is_cancel = e.to_string().contains("Fiber cancelled");
                if first_error.as_ref().is_none_or(|(c, _)| *c && !is_cancel) {
                    first_error = Some((is_cancel, e));
                }
            }
            // Already awaited through its handle inside the scope.
            None => results.push(Value::Nil),
        }
    }
    drop(state);
    body?;
    joined?;
    match first_error {
        Some((_, e)) => Err(e),
        None => Ok(Value::list(results)),
    }
}

pub fn call(rt: &mut dyn Runtime, name: &str, args: &[Value]) -> RyndResult<Value> {
    match name {
        "spawn" => {
            let id = spawn_fiber(rt, args[0].clone(), false)?;
            Ok(Value::variant("Fiber", vec![Value::Int(id)]))
        }

        "yield_fiber" => {
            let (group, me) = current();
            let mut state = lock(&group);
            if state.cancelled(me) {
                return Err(cancelled_error());
            }
            if !state.ready.is_empty() {
                state.ready.push_back(me);
                state.pass();
                drop(hold(state, me));
            }
            Ok(Value::Nil)
        }

        "await_fiber" => await_id(handle_of(&args[0], "Fiber", "Actor")?),

        "nursery" => nursery(rt, &args[0]),

        "create_mailbox" => {
            let id = next_id();
            let (group, _) = current();
            lock(&group).mailboxes.insert(id, VecDeque::new());
            Ok(Value::variant("Mailbox", vec![Value::Int(id)]))
        }

        "close_mailbox" => {
            let id = handle_of(&args[0], "Mailbox", "Actor")?;
            let (group, _) = current();
            let mut state = lock(&group);
            let removed = state.mailboxes.remove(&id).is_some();
            state.wake_waiting();
            Ok(Value::Bool(removed))
        }

        "spawn_actor" => {
            let id = next_id();
            let mailbox = Value::variant("Mailbox", vec![Value::Int(id)]);
            let body = args[0].clone();
            let task = Value::Compiled {
                name: format!("actor_{id}").into(),
                arity: 0,
                func: Rc::new(move |rt, _, _| rt.call_ref(&body, std::slice::from_ref(&mailbox))),
            };
            let (group, _) = current();
            lock(&group).mailboxes.insert(id, VecDeque::new());
            if let Err(e) = spawn_with_id(rt, task, false, id) {
                lock(&group).mailboxes.remove(&id);
                return Err(e);
            }
            Ok(Value::variant("Actor", vec![Value::Int(id)]))
        }

        "send" => {
            let id = handle_of(&args[0], "Actor", "Mailbox")?;
            let (group, _) = current();
            let mut state = lock(&group);
            let queue = state
                .mailboxes
                .get_mut(&id)
                .ok_or_else(|| error("Target Actor/Mailbox does not exist"))?;
            queue.push_back(args[1].clone());
            state.wake_waiting();
            Ok(Value::Bool(true))
        }

        "receive" | "receive_timeout" => {
            let id = handle_of(&args[0], "Mailbox", "Actor")?;
            let deadline = if name == "receive_timeout" {
                let ms = match &args[1] {
                    Value::Int(n) => {
                        u64::try_from(*n).map_err(|_| error("Timeout must be non-negative"))?
                    }
                    _ => return Err(error("Expected integer milliseconds")),
                };
                Some(Instant::now() + Duration::from_millis(ms))
            } else {
                None
            };
            let message = RefCell::new(None);
            let got = park(
                |state| match state.mailboxes.get_mut(&id) {
                    Some(queue) => match queue.pop_front() {
                        Some(msg) => {
                            *message.borrow_mut() = Some(msg);
                            true
                        }
                        None => false,
                    },
                    None => true,
                },
                deadline,
                "receive on an empty mailbox",
            )?;
            let message = message.into_inner();
            match (name, got, message) {
                ("receive", _, Some(msg)) => Ok(msg),
                ("receive", _, None) => Err(error("Mailbox does not exist")),
                (_, true, Some(msg)) => Ok(Value::variant("Some", vec![msg])),
                (_, true, None) => Err(error("Mailbox does not exist")),
                (_, false, _) => Ok(Value::variant("None", vec![])),
            }
        }

        "self_id" => {
            let (_, me) = current();
            // The program's first thread is fiber 0; spawned fibers and actors
            // each get their own id, so `self_id` composes with `await_fiber`.
            Ok(Value::variant("Fiber", vec![Value::Int(me)]))
        }

        "channel" => {
            let id = next_id();
            let (group, _) = current();
            lock(&group).channels.insert(id, VecDeque::new());
            Ok(Value::tuple(vec![
                Value::variant("Sender", vec![Value::Int(id)]),
                Value::variant("Receiver", vec![Value::Int(id)]),
            ]))
        }

        "channel_send" => {
            let id = extract_handle(&args[0], "Sender")?;
            let (group, _) = current();
            let mut state = lock(&group);
            let queue = state
                .channels
                .get_mut(&id)
                .ok_or_else(|| error("Channel sender is disconnected"))?;
            queue.push_back(args[1].clone());
            state.wake_waiting();
            Ok(Value::Bool(true))
        }

        "channel_recv" => {
            let id = extract_handle(&args[0], "Receiver")?;
            let message = RefCell::new(None);
            park(
                |state| match state.channels.get_mut(&id) {
                    Some(queue) => match queue.pop_front() {
                        Some(msg) => {
                            *message.borrow_mut() = Some(msg);
                            true
                        }
                        None => false,
                    },
                    None => true,
                },
                None,
                "channel_recv on an empty channel",
            )?;
            message
                .into_inner()
                .ok_or_else(|| error("Channel receiver is disconnected"))
        }

        "channel_try_recv" => {
            let id = extract_handle(&args[0], "Receiver")?;
            let (group, _) = current();
            let mut state = lock(&group);
            let queue = state
                .channels
                .get_mut(&id)
                .ok_or_else(|| error("Channel receiver is disconnected"))?;
            Ok(match queue.pop_front() {
                Some(msg) => Value::variant("Some", vec![msg]),
                None => Value::variant("None", vec![]),
            })
        }

        "close_channel" => {
            // Either side closes the channel: pending messages are released and
            // blocked receivers wake with a disconnect error.
            let id = handle_of(&args[0], "Sender", "Receiver")?;
            let (group, _) = current();
            let mut state = lock(&group);
            let removed = state.channels.remove(&id).is_some();
            state.wake_waiting();
            Ok(Value::Bool(removed))
        }

        "is_channel_closed" => {
            let id = handle_of(&args[0], "Sender", "Receiver")?;
            let (group, _) = current();
            let open = lock(&group).channels.contains_key(&id);
            Ok(Value::Bool(!open))
        }

        _ => Err(error(format!("Unknown concurrency builtin '{name}'"))),
    }
}
