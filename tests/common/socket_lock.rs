//! Cross-test-binary serialization for socket-using tests.
//!
//! `cargo test` runs each integration test file in a separate process, so an
//! in-process mutex does not prevent two binaries from simultaneously binding
//! ephemeral ports, exhausting file descriptors, or contending with the kernel's
//! TCP backlog. When two binaries both create listeners on `127.0.0.1` with
//! port 0 and race to `connect`, the kernel can drop SYN-ACKs and one of the
//! `connect` calls hangs long enough to fail. The lock below serializes
//! socket-using tests across the whole `cargo test` invocation by holding an
//! exclusive `flock(2)` on a single shared file. Acquire it at the top of any
//! socket-using test before touching TCP.

#![allow(unsafe_code)]

use std::fs::OpenOptions;
use std::io::{Error, Result};
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;

const LOCK_EX: i32 = 2;
const LOCK_UN: i32 = 8;
const LOCK_FILE_NAME: &str = "rynd-socket-tests.lock";

/// Acquire the shared socket-test lock for the duration of the returned guard.
/// Drop the guard (or let it go out of scope) to release it for the next test.
pub fn acquire() -> Result<SocketLock> {
    let path = lock_path();
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)?;
    let fd = file.as_raw_fd();
    let result = unsafe { flock(fd, LOCK_EX) };
    if result != 0 {
        return Err(Error::other(format!(
            "flock LOCK_EX failed: errno={result}"
        )));
    }
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    std::mem::forget(file);
    Ok(SocketLock { fd: Some(owned) })
}

pub struct SocketLock {
    fd: Option<OwnedFd>,
}

impl Drop for SocketLock {
    fn drop(&mut self) {
        if let Some(fd) = self.fd.take() {
            let raw = fd.as_raw_fd();
            unsafe {
                flock(raw, LOCK_UN);
            }
        }
    }
}

fn lock_path() -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_TMPDIR")
        .or_else(|| std::env::var_os("TMPDIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join(LOCK_FILE_NAME)
}

unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
}
