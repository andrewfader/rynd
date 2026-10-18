//! Fibers, structured nurseries, actors, and channels for Rynd.
//!
//! Fibers are stackful coroutines on the thread that runs the program (see
//! `fiber` for stacks and switching), so a fiber can suspend anywhere: inside
//! a `map` callback, deep in recursion, or in a socket read. A fiber runs
//! until it waits — `await_fiber`, `receive`, `channel_recv`, nursery joins,
//! `yield_fiber`, `sleep_ms`, or socket I/O. Scheduling is cooperative: code
//! between two waits is never interleaved with another fiber, and a long
//! computation should call `yield_fiber()` now and then.
//!
//! Sockets are non-blocking and wait in one `poll(2)` loop. The few
//! operations with no non-blocking form (name resolution and connecting,
//! child processes, stdin) run on a helper thread while their fiber waits.
//! Script values never leave the program's thread.
use super::{
    fiber,
    runtime::{Runtime, error},
    value::Value,
};
use crate::error::RyndResult;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, VecDeque};
use std::convert::TryFrom;
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

pub static BUILTINS: &[(&str, usize)] = &[
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

/// Address space reserved for each fiber's stack; pages are committed only
/// as recursion actually touches them.
const FIBER_STACK_BYTES: usize = 256 * 1024 * 1024;
/// Stacks of finished fibers kept for reuse (each keeps at most its top
/// 64 KiB of touched pages).
const STACK_POOL: usize = 256;
/// How often busy fibers check for finished I/O and timers.
const POLL_INTERVAL: Duration = Duration::from_millis(1);
/// Longest single `poll` while a run has limits, so interrupts are noticed.
const LIMITED_POLL_MS: i32 = 50;

static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn next_id() -> i64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// A script condition a parked fiber waits for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Key {
    Fiber(i64),
    Mailbox(i64),
    Channel(i64),
}

enum Wait<'a> {
    Keys(&'a [Key]),
    Io { fd: i32, events: i16 },
    Sleep,
}

/// Why a parked fiber resumed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Wake {
    Ready,
    Timeout,
    Cancelled,
    Deadlock,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Ready,
    Running,
    Parked,
    Done,
}

struct Fiber {
    /// Saved stack pointer while the fiber is not running.
    sp: usize,
    /// `None` for fiber 0, which runs on the thread's own stack.
    stack: Option<fiber::Stack>,
    entry: Option<Box<dyn FnOnce()>>,
    /// The `safety` stack bounds in effect while this fiber runs.
    claim: (usize, usize),
    status: Status,
    /// Bumped at every park, so stale wake-ups (timers, I/O) are ignored.
    token: u64,
    wake: Wake,
    /// Waiting for a helper thread, which still uses its stack.
    offloaded: bool,
    /// Registrations of the current park, removed when it wakes.
    keys: Vec<Key>,
    io_wait: bool,
    timer: bool,
    cancelled: bool,
    /// Detached fibers (server connections) drop their result on completion.
    detached: bool,
    result: Option<RyndResult<Value>>,
}

impl Fiber {
    fn new(stack: Option<fiber::Stack>, sp: usize, status: Status) -> Self {
        Fiber {
            sp,
            stack,
            entry: None,
            claim: (0, 0),
            status,
            token: 0,
            wake: Wake::Ready,
            offloaded: false,
            keys: Vec::new(),
            io_wait: false,
            timer: false,
            cancelled: false,
            detached: false,
            result: None,
        }
    }
}

struct IoWait {
    fd: i32,
    events: i16,
    id: i64,
    token: u64,
}

/// Completions posted by helper threads, and a pipe that wakes `poll`.
#[cfg(unix)]
struct Notifier {
    done: std::sync::Mutex<Vec<i64>>,
    writer: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
impl Notifier {
    fn complete(&self, id: i64) {
        self.done.lock().unwrap_or_else(|p| p.into_inner()).push(id);
        let _ = std::io::Write::write(&mut &self.writer, &[1]);
    }
}

/// One thread's fibers and everything they wait on.
struct Scheduler {
    current: i64,
    fibers: HashMap<i64, Box<Fiber>>,
    ready: VecDeque<i64>,
    /// Unfinished fibers other than fiber 0.
    live: usize,
    waiters: HashMap<Key, Vec<(i64, u64)>>,
    /// Deadlines of parked fibers; entries whose park ended stay until they
    /// reach the top, so `live_timers` counts the real ones.
    timers: BinaryHeap<Reverse<(Instant, u64, i64)>>,
    live_timers: usize,
    io: Vec<IoWait>,
    offloads: usize,
    #[cfg(unix)]
    notifier: Option<(std::sync::Arc<Notifier>, std::os::unix::net::UnixStream)>,
    mailboxes: HashMap<i64, VecDeque<Value>>,
    channels: HashMap<i64, VecDeque<Value>>,
    /// The stack of a fiber that just finished; freed once off it.
    finished: Option<fiber::Stack>,
    /// Where a finished fiber's final switch stores its stack pointer.
    discard: usize,
    pool: Vec<fiber::Stack>,
    last_poll: Instant,
}

thread_local! {
    static SCHEDULER: RefCell<Option<Box<Scheduler>>> = const { RefCell::new(None) };
}

fn with<T>(f: impl FnOnce(&mut Scheduler) -> T) -> T {
    SCHEDULER.with(|cell| {
        let mut slot = cell.borrow_mut();
        f(slot.get_or_insert_with(|| Box::new(Scheduler::new())))
    })
}

/// Run `f` on this thread's scheduler if one exists and is not in use.
fn peek<T>(f: impl FnOnce(&mut Scheduler) -> T) -> Option<T> {
    SCHEDULER
        .try_with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(|s| f(s)))
        .ok()
        .flatten()
}

impl Scheduler {
    fn new() -> Self {
        let mut fibers = HashMap::new();
        // Fiber 0 is whatever already runs on this thread.
        fibers.insert(0, Box::new(Fiber::new(None, 0, Status::Running)));
        Scheduler {
            current: 0,
            fibers,
            ready: VecDeque::new(),
            live: 0,
            waiters: HashMap::new(),
            timers: BinaryHeap::new(),
            live_timers: 0,
            io: Vec::new(),
            offloads: 0,
            #[cfg(unix)]
            notifier: None,
            mailboxes: HashMap::new(),
            channels: HashMap::new(),
            finished: None,
            discard: 0,
            pool: Vec::new(),
            last_poll: Instant::now(),
        }
    }

    fn me(&mut self) -> &mut Fiber {
        let id = self.current;
        self.fibers.get_mut(&id).expect("running fiber")
    }

    /// Whether anything could still wake a parked fiber.
    fn pending(&self) -> bool {
        !self.ready.is_empty() || !self.io.is_empty() || self.live_timers > 0 || self.offloads > 0
    }

    /// Make a parked fiber runnable, unless the wake-up is stale, and drop
    /// its other registrations.
    fn wake(&mut self, id: i64, token: u64, why: Wake) -> bool {
        let Some(fiber) = self.fibers.get_mut(&id) else {
            return false;
        };
        if fiber.status != Status::Parked || fiber.token != token || fiber.offloaded {
            return false;
        }
        fiber.status = Status::Ready;
        fiber.wake = why;
        let keys = std::mem::take(&mut fiber.keys);
        let io_wait = std::mem::replace(&mut fiber.io_wait, false);
        if std::mem::replace(&mut fiber.timer, false) {
            self.live_timers -= 1;
        }
        self.ready.push_back(id);
        for key in keys {
            if let Some(list) = self.waiters.get_mut(&key) {
                list.retain(|(waiter, _)| *waiter != id);
                if list.is_empty() {
                    self.waiters.remove(&key);
                }
            }
        }
        if io_wait {
            self.io.retain(|w| w.id != id);
        }
        true
    }

    fn timer_live(&self, token: u64, id: i64) -> bool {
        self.fibers
            .get(&id)
            .is_some_and(|f| f.timer && f.token == token && f.status == Status::Parked)
    }

    /// The earliest live deadline, discarding ended ones on top.
    fn next_deadline(&mut self) -> Option<Instant> {
        while let Some(Reverse((when, token, id))) = self.timers.peek().copied() {
            if self.timer_live(token, id) {
                return Some(when);
            }
            self.timers.pop();
        }
        None
    }

    fn wake_key(&mut self, key: Key) {
        for (id, token) in self.waiters.remove(&key).unwrap_or_default() {
            self.wake(id, token, Wake::Ready);
        }
    }

    fn cancel(&mut self, id: i64) {
        let Some(fiber) = self.fibers.get_mut(&id) else {
            return;
        };
        if fiber.status != Status::Done {
            fiber.cancelled = true;
            let token = fiber.token;
            self.wake(id, token, Wake::Cancelled);
        }
    }

    fn pop_ready(&mut self) -> Option<i64> {
        while let Some(id) = self.ready.pop_front() {
            if self
                .fibers
                .get(&id)
                .is_some_and(|f| f.status == Status::Ready)
            {
                return Some(id);
            }
        }
        None
    }

    fn expire_timers(&mut self, now: Instant) {
        while let Some(when) = self.next_deadline() {
            if when > now {
                break;
            }
            let Reverse((_, token, id)) = self.timers.pop().expect("timer");
            self.wake(id, token, Wake::Timeout);
        }
    }

    /// Nothing can ever wake anyone: resume a parked fiber with a deadlock
    /// error, the current one or fiber 0 first.
    fn break_deadlock(&mut self) -> bool {
        let parked = |f: &Fiber| f.status == Status::Parked && !f.offloaded;
        let mut candidates: Vec<i64> = self
            .fibers
            .iter()
            .filter(|(_, f)| parked(f))
            .map(|(id, _)| *id)
            .collect();
        candidates.sort_by_key(|id| (*id != self.current, *id != 0, *id));
        match candidates.first() {
            Some(&id) => {
                let token = self.fibers[&id].token;
                self.wake(id, token, Wake::Deadlock)
            }
            None => false,
        }
    }

    fn recycle(&mut self, stack: fiber::Stack) {
        if self.pool.len() < STACK_POOL {
            stack.release_pages();
            self.pool.push(stack);
        }
    }

    #[cfg(unix)]
    fn notifier(&mut self) -> std::io::Result<std::sync::Arc<Notifier>> {
        if self.notifier.is_none() {
            let (reader, writer) = std::os::unix::net::UnixStream::pair()?;
            reader.set_nonblocking(true)?;
            let notifier = std::sync::Arc::new(Notifier {
                done: std::sync::Mutex::new(Vec::new()),
                writer,
            });
            self.notifier = Some((notifier, reader));
        }
        Ok(self.notifier.as_ref().unwrap().0.clone())
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        // At thread exit, suspended fibers can never resume or unwind; leak
        // their frames and queued values instead of running destructors
        // during thread-local teardown.
        for (_, fiber) in self.fibers.drain() {
            std::mem::forget(fiber);
        }
        std::mem::forget(std::mem::take(&mut self.mailboxes));
        std::mem::forget(std::mem::take(&mut self.channels));
    }
}

fn cancelled_error() -> crate::error::RyndError {
    error("Fiber cancelled")
}

fn deadlock_error(what: &str) -> crate::error::RyndError {
    error(format!(
        "Deadlock: {what} would wait forever (no other fiber can run)"
    ))
}

// ---------------------------------------------------------------- switching

/// Entry point of every new fiber's stack.
extern "C" fn fiber_main(_: usize) -> ! {
    resumed();
    let (entry, size) = with(|s| {
        let me = s.me();
        (
            me.entry.take(),
            me.stack.as_ref().map_or(0, |stack| stack.size()),
        )
    });
    super::safety::claim_stack(size);
    // `run_task` catches script panics; anything else is a bug here.
    if entry
        .is_some_and(|entry| std::panic::catch_unwind(std::panic::AssertUnwindSafe(entry)).is_err())
    {
        eprintln!("rynd: fiber entry panicked");
        std::process::abort();
    }
    finish()
}

/// Bookkeeping after any switch lands on this fiber.
fn resumed() {
    let (stack, claim) = with(|s| (s.finished.take(), s.me().claim));
    if let Some(stack) = stack {
        with(|s| s.recycle(stack));
    }
    super::safety::set_stack_claim(claim);
}

/// Switch from the current fiber to `next`; returns when switched back.
fn switch_to(next: i64, finished: bool) {
    let claim = super::safety::stack_claim();
    let (from, to) = with(|s| {
        let from: *mut usize = if finished {
            std::ptr::addr_of_mut!(s.discard)
        } else {
            let me = s.me();
            me.claim = claim;
            std::ptr::addr_of_mut!(me.sp)
        };
        s.current = next;
        let target = s.fibers.get_mut(&next).expect("next fiber");
        target.status = Status::Running;
        (from, target.sp)
    });
    // SAFETY: `to` was saved by a switch away from `next` (or prepared for
    // its first run) and nothing else resumes it; `from` points into a boxed
    // fiber or the boxed scheduler, which outlive the switch.
    unsafe { fiber::switch(from, to) };
    resumed();
}

/// Wait for I/O readiness, timers, or helper threads, making fibers ready.
/// Returns false if there is nothing to wait for. With `nonblocking`, only
/// collects what is already done.
fn wait_events(nonblocking: bool) -> bool {
    let now = Instant::now();
    let plan = with(|s| {
        s.last_poll = now;
        s.expire_timers(now);
        if !s.ready.is_empty() && !nonblocking {
            return None;
        }
        let next_deadline = s.next_deadline();
        if s.io.is_empty() && next_deadline.is_none() && s.offloads == 0 {
            return Some((Vec::new(), None));
        }
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut fds: Vec<fiber::PollFd> =
            s.io.iter()
                .map(|w| fiber::PollFd {
                    fd: w.fd,
                    events: w.events,
                    revents: 0,
                })
                .collect();
        #[cfg(unix)]
        if let Some((_, reader)) = &s.notifier {
            use std::os::fd::AsRawFd;
            fds.push(fiber::PollFd {
                fd: reader.as_raw_fd(),
                events: fiber::POLLIN,
                revents: 0,
            });
        }
        let mut timeout = next_deadline.map(|when| {
            let left = when.saturating_duration_since(now);
            i32::try_from(left.as_millis() + u128::from(left.subsec_nanos() % 1_000_000 != 0))
                .unwrap_or(i32::MAX)
        });
        if super::safety::limited() {
            timeout = Some(timeout.map_or(LIMITED_POLL_MS, |t| t.min(LIMITED_POLL_MS)));
        }
        if nonblocking || !s.ready.is_empty() {
            timeout = Some(0);
        }
        Some((fds, Some(timeout.unwrap_or(-1))))
    });
    let (mut fds, timeout) = match plan {
        None => return true,
        Some((_, None)) => return false,
        Some((fds, Some(timeout))) => (fds, timeout),
    };
    if let Err(e) = fiber::poll_fds(&mut fds, timeout) {
        eprintln!("rynd: poll failed: {e}");
        std::thread::sleep(Duration::from_millis(1));
    }
    with(|s| {
        for pfd in fds.iter().filter(|p| p.revents != 0) {
            if let Some(i) =
                s.io.iter()
                    .position(|w| w.fd == pfd.fd && w.events == pfd.events)
            {
                let w = s.io.swap_remove(i);
                if let Some(fiber) = s.fibers.get_mut(&w.id).filter(|f| f.token == w.token) {
                    fiber.io_wait = false;
                }
                s.wake(w.id, w.token, Wake::Ready);
            }
        }
        #[cfg(unix)]
        {
            let done = match &s.notifier {
                Some((notifier, reader)) => {
                    let mut sink = [0u8; 64];
                    while matches!(std::io::Read::read(&mut &*reader, &mut sink), Ok(n) if n > 0) {}
                    std::mem::take(&mut *notifier.done.lock().unwrap_or_else(|p| p.into_inner()))
                }
                None => Vec::new(),
            };
            for id in done {
                if let Some(fiber) = s.fibers.get_mut(&id) {
                    fiber.offloaded = false;
                    let token = fiber.token;
                    s.wake(id, token, Wake::Ready);
                }
            }
        }
        s.expire_timers(Instant::now());
    });
    true
}

/// The next fiber to run, waiting for events while none is ready.
fn pick_next() -> i64 {
    loop {
        if let Some(next) = with(|s| s.pop_ready()) {
            return next;
        }
        if !wait_events(false) && !with(|s| s.break_deadlock()) {
            panic!("rynd: fiber scheduler has nothing to run");
        }
    }
}

/// Run other fibers until the current one (parked or queued) resumes.
fn suspend() {
    let next = pick_next();
    if next == with(|s| s.current) {
        with(|s| s.me().status = Status::Running);
        return;
    }
    switch_to(next, false);
}

/// End the current fiber and switch away for good.
fn finish() -> ! {
    let result = with(|s| {
        let me = s.current;
        let fiber = s.me();
        fiber.status = Status::Done;
        let stack = fiber.stack.take();
        let detached = fiber.detached;
        s.finished = stack;
        s.live = s.live.saturating_sub(1);
        let mailbox = s.mailboxes.remove(&me);
        s.wake_key(Key::Fiber(me));
        let fiber = if detached { s.fibers.remove(&me) } else { None };
        (mailbox, fiber)
    });
    drop(result);
    loop {
        let next = pick_next();
        switch_to(next, true);
    }
}

/// Park the current fiber until `wait` is satisfied, `deadline` passes
/// (`Wake::Timeout`), or the fiber is cancelled. Waiting on a script
/// condition with nothing left that could satisfy it is a deadlock error.
fn park(wait: Wait, deadline: Option<Instant>, what: &str) -> RyndResult<Wake> {
    let budget = super::safety::deadline();
    let until = match (deadline, budget) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    with(|s| {
        if s.me().cancelled {
            return Err(cancelled_error());
        }
        if matches!(wait, Wait::Keys(_)) && deadline.is_none() && !s.pending() {
            return Err(deadlock_error(what));
        }
        let me = s.current;
        let fiber = s.me();
        fiber.token += 1;
        fiber.status = Status::Parked;
        fiber.wake = Wake::Ready;
        let token = fiber.token;
        match wait {
            Wait::Keys(keys) => {
                fiber.keys = keys.to_vec();
                for key in keys {
                    s.waiters.entry(*key).or_default().push((me, token));
                }
            }
            Wait::Io { fd, events } => {
                fiber.io_wait = true;
                s.io.push(IoWait {
                    fd,
                    events,
                    id: me,
                    token,
                });
            }
            Wait::Sleep => {}
        }
        if let Some(until) = until {
            s.me().timer = true;
            s.live_timers += 1;
            s.timers.push(Reverse((until, token, me)));
        }
        Ok(())
    })?;
    suspend();
    match with(|s| s.me().wake) {
        Wake::Cancelled => Err(cancelled_error()),
        Wake::Deadlock => Err(deadlock_error(what)),
        Wake::Timeout => {
            // A wait cut short by the run's time limit ends the run.
            super::safety::check()?;
            Ok(Wake::Timeout)
        }
        Wake::Ready => Ok(Wake::Ready),
    }
}

/// Let other ready fibers run (and collect finished I/O) before continuing.
fn yield_now() -> RyndResult<()> {
    if with(|s| s.me().cancelled) {
        return Err(cancelled_error());
    }
    let poll_due = with(|s| {
        (!s.io.is_empty() || s.live_timers > 0 || s.offloads > 0)
            && s.last_poll.elapsed() >= POLL_INTERVAL
    });
    if poll_due {
        wait_events(true);
    }
    let others = with(|s| {
        if s.ready.is_empty() {
            return false;
        }
        let me = s.current;
        s.me().status = Status::Ready;
        s.ready.push_back(me);
        true
    });
    if others {
        suspend();
    }
    if with(|s| s.me().cancelled) {
        return Err(cancelled_error());
    }
    Ok(())
}

// ---------------------------------------------------------------- waiting on the world

/// Sleep without blocking other fibers; cancellation ends the sleep early.
pub fn sleep(duration: Duration) -> RyndResult<()> {
    park(Wait::Sleep, Some(Instant::now() + duration), "sleep")?;
    Ok(())
}

/// Wait until `fd` is readable (or writable) or `deadline` passes, letting
/// other fibers run meanwhile.
pub fn wait_io(fd: i32, write: bool, deadline: Option<Instant>) -> std::io::Result<()> {
    let events = if write { fiber::POLLOUT } else { fiber::POLLIN };
    match park(Wait::Io { fd, events }, deadline, "socket I/O") {
        Ok(Wake::Timeout) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "connection timed out",
        )),
        Ok(_) => Ok(()),
        Err(e) => Err(std::io::Error::other(e.to_string())),
    }
}

/// Run `work`, which blocks the calling thread, on a helper thread so other
/// fibers keep running; returns its result. With no other fibers, runs it
/// directly.
pub fn blocking<T: Send, F: FnOnce() -> T + Send>(work: F) -> T {
    let alone = peek(|s| s.fibers.len() <= 1).unwrap_or(true);
    if alone || !fiber::SUPPORTED {
        return work();
    }
    #[cfg(unix)]
    {
        offload(work)
    }
    #[cfg(not(unix))]
    {
        work()
    }
}

#[cfg(unix)]
fn offload<T: Send, F: FnOnce() -> T + Send>(work: F) -> T {
    struct Out<T>(*mut Option<std::thread::Result<T>>);
    // SAFETY: the pointer is written once by the helper thread while the
    // owning fiber waits, and read only after the helper signals completion.
    unsafe impl<T: Send> Send for Out<T> {}

    let Ok(notifier) = with(|s| s.notifier()) else {
        return work();
    };
    let me = with(|s| s.current);
    let mut slot: Option<std::thread::Result<T>> = None;
    let out = Out(&mut slot);
    let job: Box<dyn FnOnce() + Send + '_> = Box::new(move || {
        let out = out;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
        // SAFETY: see `Out`.
        unsafe { *out.0 = Some(result) };
    });
    // SAFETY: this fiber does not return (or let anything free its stack)
    // until the helper reports completion below, so everything the job
    // borrows outlives it.
    let job: Box<dyn FnOnce() + Send + 'static> = unsafe { std::mem::transmute(job) };
    with(|s| {
        s.offloads += 1;
        let fiber = s.me();
        fiber.offloaded = true;
        fiber.token += 1;
        fiber.status = Status::Parked;
        fiber.wake = Wake::Ready;
    });
    let helper = notifier.clone();
    let spawned = std::thread::Builder::new()
        .name("rynd-io".into())
        .spawn(move || {
            job();
            helper.complete(me);
        });
    if let Err(e) = spawned {
        panic!("rynd: cannot start an I/O helper thread: {}", e);
    }
    // Only the scheduler's completion handling clears `offloaded`, after the
    // helper has published its result; never read `slot` before that.
    loop {
        suspend();
        if !with(|s| s.me().offloaded) {
            break;
        }
        with(|s| s.me().status = Status::Parked);
    }
    with(|s| s.offloads -= 1);
    match slot.take().expect("helper result") {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

// ---------------------------------------------------------------- fibers

/// Start `task` on a new fiber. The caller keeps running; the fiber starts
/// at the caller's next wait or yield.
pub fn spawn_fiber(rt: &dyn Runtime, task: Value, detached: bool) -> RyndResult<i64> {
    spawn_with_id(rt, task, detached, next_id())
}

fn spawn_with_id(rt: &dyn Runtime, task: Value, detached: bool, id: i64) -> RyndResult<i64> {
    if !fiber::SUPPORTED {
        return Err(error("Fibers are not supported on this platform"));
    }
    if let Some(max) = super::safety::fiber_limit().filter(|max| with(|s| s.live) >= *max) {
        return Err(super::safety::fibers_exceeded(max));
    }
    let child = rt.fork()?;
    let stack = match with(|s| s.pool.pop()) {
        Some(stack) => stack,
        None => fiber::Stack::new(FIBER_STACK_BYTES)
            .map_err(|e| error(format!("spawn(): cannot allocate a fiber stack: {e}")))?,
    };
    let sp = fiber::prepare(&stack, fiber_main, 0);
    let mut fiber = Fiber::new(Some(stack), sp, Status::Ready);
    fiber.detached = detached;
    fiber.entry = Some(Box::new(move || run_task(id, child, task)));
    with(|s| {
        s.fibers.insert(id, Box::new(fiber));
        s.ready.push_back(id);
        s.live += 1;
    });
    Ok(id)
}

fn run_task(id: i64, mut rt: Box<dyn Runtime>, task: Value) {
    let result = if with(|s| s.me().cancelled) {
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
    drop(task);
    drop(rt);
    let (detached, cancelled) = with(|s| {
        let me = s.me();
        (me.detached, me.cancelled)
    });
    if detached {
        if let (Err(e), false) = (&result, cancelled) {
            eprintln!("rynd: fiber {id} failed: {e}");
        }
        drop(result);
    } else {
        with(|s| s.me().result = Some(result));
    }
}

fn await_id(id: i64) -> RyndResult<Value> {
    if id == with(|s| s.current) {
        return Err(error("A fiber cannot await itself"));
    }
    loop {
        let done = with(|s| match s.fibers.get(&id) {
            None => Some(Err(error("Invalid or expired Fiber handle"))),
            Some(f) if f.result.is_some() => {
                let mut fiber = s.fibers.remove(&id).expect("finished fiber");
                Some(fiber.result.take().expect("fiber result"))
            }
            Some(_) => None,
        });
        if let Some(result) = done {
            return result;
        }
        park(Wait::Keys(&[Key::Fiber(id)]), None, "await_fiber")?;
    }
}

fn extract_handle(value: &Value, expected: &str) -> RyndResult<i64> {
    match value {
        Value::Variant(v) if &*v.name == expected && v.values.len() == 1 => match v.values[0] {
            Value::Int(id) => Ok(id),
            _ => Err(error(format!("Invalid {expected} handle"))),
        },
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
    let spawn_fn = Value::compiled("spawn", 1, move |rt, args, _| {
        let id = spawn_fiber(rt, args[0].clone(), false)?;
        spawned.borrow_mut().push(id);
        Ok(Value::variant("Fiber", vec![Value::Int(id)]))
    });
    let mut scope = BTreeMap::new();
    scope.insert("id".into(), Value::Int(next_id()));
    scope.insert("spawn".into(), spawn_fn);
    let body = rt.call_ref(block, &[Value::map(scope)]);
    let ids = children.borrow().clone();
    let keys: Vec<Key> = ids.iter().map(|id| Key::Fiber(*id)).collect();
    let failed = |s: &Scheduler| {
        ids.iter()
            .any(|id| matches!(s.fibers.get(id), Some(f) if matches!(f.result, Some(Err(_)))))
    };
    let done = |s: &Scheduler| {
        ids.iter()
            .all(|id| s.fibers.get(id).is_none_or(|f| f.result.is_some()))
    };
    // Children never outlive the scope: wait for all of them, canceling the
    // rest as soon as the body or any child fails.
    let mut joined = Ok(());
    if body.is_ok() {
        while !with(|s| done(s) || failed(s)) {
            if let Err(e) = park(Wait::Keys(&keys), None, "nursery") {
                joined = Err(e);
                break;
            }
        }
    }
    if body.is_err() || joined.is_err() || with(|s| failed(s)) {
        with(|s| {
            for id in &ids {
                s.cancel(*id);
            }
        });
        while !with(|s| done(s)) {
            if let Err(e) = park(Wait::Keys(&keys), None, "nursery cancellation") {
                joined = joined.and(Err(e));
                break;
            }
        }
    }
    let slots: Vec<Option<RyndResult<Value>>> = with(|s| {
        ids.iter()
            .map(|id| match s.fibers.get(id) {
                Some(f) if f.result.is_some() => {
                    s.fibers.remove(id).and_then(|mut f| f.result.take())
                }
                _ => None,
            })
            .collect()
    });
    let mut results = Vec::with_capacity(ids.len());
    let mut first_error = None;
    for slot in slots {
        match slot {
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
    body?;
    joined?;
    match first_error {
        Some((_, e)) => Err(e),
        None => Ok(Value::list(results)),
    }
}

/// Wait until `take` produces a message, the deadline passes (`None`), or
/// the queue it reads is closed (`Some(None)` from `take`).
fn receive_from(
    key: Key,
    deadline: Option<Instant>,
    what: &str,
    take: impl Fn(&mut Scheduler) -> Option<Option<Value>>,
) -> RyndResult<Option<Option<Value>>> {
    loop {
        if let Some(got) = with(&take) {
            return Ok(Some(got));
        }
        if park(Wait::Keys(&[key]), deadline, what)? == Wake::Timeout {
            return Ok(with(&take));
        }
    }
}

pub fn call(rt: &mut dyn Runtime, name: &str, args: &[Value]) -> RyndResult<Value> {
    match name {
        "spawn" => {
            let id = spawn_fiber(rt, args[0].clone(), false)?;
            Ok(Value::variant("Fiber", vec![Value::Int(id)]))
        }

        "yield_fiber" => {
            yield_now()?;
            Ok(Value::Nil)
        }

        "await_fiber" => await_id(handle_of(&args[0], "Fiber", "Actor")?),

        "nursery" => nursery(rt, &args[0]),

        "create_mailbox" => {
            let id = next_id();
            with(|s| s.mailboxes.insert(id, VecDeque::new()));
            Ok(Value::variant("Mailbox", vec![Value::Int(id)]))
        }

        "close_mailbox" => {
            let id = handle_of(&args[0], "Mailbox", "Actor")?;
            let removed = with(|s| {
                let removed = s.mailboxes.remove(&id);
                s.wake_key(Key::Mailbox(id));
                removed
            });
            Ok(Value::Bool(removed.is_some()))
        }

        "spawn_actor" => {
            let id = next_id();
            let mailbox = Value::variant("Mailbox", vec![Value::Int(id)]);
            let body = args[0].clone();
            let task = Value::compiled(format!("actor_{id}"), 0, move |rt, _, _| {
                rt.call_ref(&body, std::slice::from_ref(&mailbox))
            });
            with(|s| s.mailboxes.insert(id, VecDeque::new()));
            if let Err(e) = spawn_with_id(rt, task, false, id) {
                with(|s| s.mailboxes.remove(&id));
                return Err(e);
            }
            Ok(Value::variant("Actor", vec![Value::Int(id)]))
        }

        "send" => {
            let id = handle_of(&args[0], "Actor", "Mailbox")?;
            with(|s| {
                let queue = s
                    .mailboxes
                    .get_mut(&id)
                    .ok_or_else(|| error("Target Actor/Mailbox does not exist"))?;
                queue.push_back(args[1].clone());
                s.wake_key(Key::Mailbox(id));
                Ok(Value::Bool(true))
            })
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
            let got = receive_from(
                Key::Mailbox(id),
                deadline,
                "receive on an empty mailbox",
                |s| match s.mailboxes.get_mut(&id) {
                    Some(queue) => queue.pop_front().map(Some),
                    None => Some(None),
                },
            )?;
            match (name, got) {
                ("receive", Some(Some(msg))) => Ok(msg),
                (_, Some(None)) => Err(error("Mailbox does not exist")),
                (_, Some(Some(msg))) => Ok(Value::variant("Some", vec![msg])),
                ("receive", None) => Err(error("Mailbox does not exist")),
                (_, None) => Ok(Value::variant("None", vec![])),
            }
        }

        "self_id" => {
            // The program's first fiber is 0; spawned fibers and actors each
            // get their own id, so `self_id` composes with `await_fiber`.
            let me = with(|s| s.current);
            Ok(Value::variant("Fiber", vec![Value::Int(me)]))
        }

        "channel" => {
            let id = next_id();
            with(|s| s.channels.insert(id, VecDeque::new()));
            Ok(Value::tuple(vec![
                Value::variant("Sender", vec![Value::Int(id)]),
                Value::variant("Receiver", vec![Value::Int(id)]),
            ]))
        }

        "channel_send" => {
            let id = extract_handle(&args[0], "Sender")?;
            with(|s| {
                let queue = s
                    .channels
                    .get_mut(&id)
                    .ok_or_else(|| error("Channel sender is disconnected"))?;
                queue.push_back(args[1].clone());
                s.wake_key(Key::Channel(id));
                Ok(Value::Bool(true))
            })
        }

        "channel_recv" => {
            let id = extract_handle(&args[0], "Receiver")?;
            let got = receive_from(
                Key::Channel(id),
                None,
                "channel_recv on an empty channel",
                |s| match s.channels.get_mut(&id) {
                    Some(queue) => queue.pop_front().map(Some),
                    None => Some(None),
                },
            )?;
            got.flatten()
                .ok_or_else(|| error("Channel receiver is disconnected"))
        }

        "channel_try_recv" => {
            let id = extract_handle(&args[0], "Receiver")?;
            with(|s| {
                let queue = s
                    .channels
                    .get_mut(&id)
                    .ok_or_else(|| error("Channel receiver is disconnected"))?;
                Ok(match queue.pop_front() {
                    Some(msg) => Value::variant("Some", vec![msg]),
                    None => Value::variant("None", vec![]),
                })
            })
        }

        "close_channel" => {
            // Either side closes the channel: pending messages are released and
            // blocked receivers wake with a disconnect error.
            let id = handle_of(&args[0], "Sender", "Receiver")?;
            let removed = with(|s| {
                let removed = s.channels.remove(&id);
                s.wake_key(Key::Channel(id));
                removed
            });
            Ok(Value::Bool(removed.is_some()))
        }

        "is_channel_closed" => {
            let id = handle_of(&args[0], "Sender", "Receiver")?;
            Ok(Value::Bool(!with(|s| s.channels.contains_key(&id))))
        }

        _ => Err(error(format!("Unknown concurrency builtin '{name}'"))),
    }
}
