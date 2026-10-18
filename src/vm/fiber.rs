//! Fiber stacks, context switches, and `poll(2)`: the platform-specific
//! layer under the scheduler in `concurrency`.
//!
//! A fiber is a stack plus a saved stack pointer. Switching saves the
//! callee-saved registers on the current stack, stores its stack pointer,
//! loads the other fiber's, and restores that fiber's registers. Stacks are
//! anonymous mappings reserved up front and committed by the kernel only as
//! pages are touched, with an inaccessible guard region below them.
//!
//! Supported: x86-64 and AArch64 on Linux and macOS. Elsewhere [`SUPPORTED`]
//! is false and the scheduler refuses to start fibers.

#[cfg(all(
    any(target_os = "linux", target_os = "macos"),
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod platform {
    use std::ffi::{c_int, c_void};
    use std::io;

    pub const SUPPORTED: bool = true;

    #[cfg(target_os = "linux")]
    const MAP_FLAGS: c_int = 0x02 /* PRIVATE */ | 0x20 /* ANONYMOUS */ | 0x4000 /* NORESERVE */;
    #[cfg(target_os = "macos")]
    const MAP_FLAGS: c_int = 0x0002 /* PRIVATE */ | 0x1000 /* ANON */ | 0x40 /* NORESERVE */;
    const PROT_NONE: c_int = 0;
    const PROT_READ_WRITE: c_int = 1 | 2;
    #[cfg(target_os = "linux")]
    const MADV_DONTNEED: c_int = 4;

    #[cfg(target_os = "linux")]
    pub type Nfds = std::ffi::c_ulong;
    #[cfg(target_os = "macos")]
    pub type Nfds = std::ffi::c_uint;

    #[repr(C)]
    #[derive(Clone, Copy, Debug)]
    pub struct PollFd {
        pub fd: c_int,
        pub events: i16,
        pub revents: i16,
    }

    pub const POLLIN: i16 = 0x1;
    pub const POLLOUT: i16 = 0x4;

    unsafe extern "C" {
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: c_int,
            flags: c_int,
            fd: c_int,
            offset: i64,
        ) -> *mut c_void;
        fn munmap(addr: *mut c_void, len: usize) -> c_int;
        fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
        #[cfg(target_os = "linux")]
        fn madvise(addr: *mut c_void, len: usize, advice: c_int) -> c_int;
        fn poll(fds: *mut PollFd, nfds: Nfds, timeout: c_int) -> c_int;
    }

    /// Bytes below each stack that fault on access (a multiple of every
    /// supported page size).
    const GUARD: usize = 64 * 1024;
    /// Touched pages a pooled stack keeps when it is recycled.
    #[cfg(target_os = "linux")]
    const KEEP: usize = 64 * 1024;

    pub struct Stack {
        base: *mut u8,
        len: usize,
    }

    impl Stack {
        /// Reserve `size` usable bytes plus a guard region.
        pub fn new(size: usize) -> io::Result<Stack> {
            let len = size + GUARD;
            // SAFETY: a fresh private anonymous mapping aliases nothing.
            let base =
                unsafe { mmap(std::ptr::null_mut(), len, PROT_READ_WRITE, MAP_FLAGS, -1, 0) };
            if base as isize == -1 {
                return Err(io::Error::last_os_error());
            }
            let stack = Stack {
                base: base.cast(),
                len,
            };
            // SAFETY: the guard is the low end of the mapping made above.
            if unsafe { mprotect(base, GUARD, PROT_NONE) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(stack)
        }
        /// The highest address of the stack (it grows down from here).
        pub fn top(&self) -> usize {
            self.base as usize + self.len
        }
        /// Bytes usable above the guard.
        pub fn size(&self) -> usize {
            self.len - GUARD
        }
        /// Return deep pages to the kernel before the stack is reused.
        pub fn release_pages(&self) {
            #[cfg(target_os = "linux")]
            // SAFETY: the range lies inside this mapping, and no live frame
            // uses it: the stack's fiber has finished.
            unsafe {
                madvise(
                    self.base.add(GUARD).cast(),
                    self.size() - KEEP,
                    MADV_DONTNEED,
                );
            }
        }
    }

    impl Drop for Stack {
        fn drop(&mut self) {
            // SAFETY: the mapping is ours and no fiber runs on it any more.
            unsafe {
                munmap(self.base.cast(), self.len);
            }
        }
    }

    /// Wait for readiness on `fds`; `timeout` in milliseconds, -1 forever.
    pub fn poll_fds(fds: &mut [PollFd], timeout: c_int) -> io::Result<usize> {
        // SAFETY: `fds` is a valid, exclusively borrowed array of pollfd.
        let n = unsafe { poll(fds.as_mut_ptr(), fds.len() as Nfds, timeout) };
        if n < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(0);
            }
            return Err(error);
        }
        Ok(n as usize)
    }

    #[cfg(target_arch = "x86_64")]
    mod arch {
        /// Initial frame: MXCSR/x87 control words, r15, r14, r13 (entry),
        /// r12 (argument), rbx, rbp, and the trampoline as return address.
        pub const FRAME: usize = 64;

        pub unsafe fn write_frame(sp: *mut usize, entry: usize, arg: usize) {
            let trampoline: unsafe extern "C" fn() -> ! = trampoline;
            // SAFETY: the caller provides FRAME writable bytes at `sp`.
            unsafe {
                sp.write(0x1F80 | (0x037F << 32));
                sp.add(1).write(0);
                sp.add(2).write(0);
                sp.add(3).write(entry);
                sp.add(4).write(arg);
                sp.add(5).write(0);
                sp.add(6).write(0);
                sp.add(7).write(trampoline as usize);
            }
        }

        #[unsafe(naked)]
        pub unsafe extern "C" fn switch(_from: *mut usize, _to: usize) {
            std::arch::naked_asm!(
                "push rbp",
                "push rbx",
                "push r12",
                "push r13",
                "push r14",
                "push r15",
                "sub rsp, 8",
                "stmxcsr [rsp]",
                "fnstcw [rsp + 4]",
                "mov [rdi], rsp",
                "mov rsp, rsi",
                "ldmxcsr [rsp]",
                "fldcw [rsp + 4]",
                "add rsp, 8",
                "pop r15",
                "pop r14",
                "pop r13",
                "pop r12",
                "pop rbx",
                "pop rbp",
                "ret",
            )
        }

        #[unsafe(naked)]
        unsafe extern "C" fn trampoline() -> ! {
            std::arch::naked_asm!("mov rdi, r12", "call r13", "ud2")
        }
    }

    #[cfg(target_arch = "aarch64")]
    mod arch {
        /// Initial frame: x19 (argument), x20 (entry), x21-x28, x29 (zero
        /// frame pointer), x30 (the trampoline), and d8-d15.
        pub const FRAME: usize = 176;

        pub unsafe fn write_frame(sp: *mut usize, entry: usize, arg: usize) {
            let trampoline: unsafe extern "C" fn() -> ! = trampoline;
            // SAFETY: the caller provides FRAME writable bytes at `sp`.
            unsafe {
                for i in 0..FRAME / 8 {
                    sp.add(i).write(0);
                }
                sp.write(arg);
                sp.add(1).write(entry);
                sp.add(11).write(trampoline as usize);
            }
        }

        #[unsafe(naked)]
        pub unsafe extern "C" fn switch(_from: *mut usize, _to: usize) {
            std::arch::naked_asm!(
                "sub sp, sp, #176",
                "stp x19, x20, [sp, #0]",
                "stp x21, x22, [sp, #16]",
                "stp x23, x24, [sp, #32]",
                "stp x25, x26, [sp, #48]",
                "stp x27, x28, [sp, #64]",
                "stp x29, x30, [sp, #80]",
                "stp d8, d9, [sp, #96]",
                "stp d10, d11, [sp, #112]",
                "stp d12, d13, [sp, #128]",
                "stp d14, d15, [sp, #144]",
                "mov x9, sp",
                "str x9, [x0]",
                "mov sp, x1",
                "ldp x19, x20, [sp, #0]",
                "ldp x21, x22, [sp, #16]",
                "ldp x23, x24, [sp, #32]",
                "ldp x25, x26, [sp, #48]",
                "ldp x27, x28, [sp, #64]",
                "ldp x29, x30, [sp, #80]",
                "ldp d8, d9, [sp, #96]",
                "ldp d10, d11, [sp, #112]",
                "ldp d12, d13, [sp, #128]",
                "ldp d14, d15, [sp, #144]",
                "add sp, sp, #176",
                "ret",
            )
        }

        #[unsafe(naked)]
        unsafe extern "C" fn trampoline() -> ! {
            std::arch::naked_asm!("mov x0, x19", "blr x20", "brk #1")
        }
    }

    /// Lay out `stack` so that the first switch to the returned stack
    /// pointer calls `entry(arg)` on it. `entry` must never return.
    pub fn prepare(stack: &Stack, entry: extern "C" fn(usize) -> !, arg: usize) -> usize {
        // 16-byte aligned once the frame is popped, as both ABIs require.
        let top = (stack.top() - 16) & !15;
        let sp = top - arch::FRAME;
        // SAFETY: `sp..top` lies inside the stack's writable region.
        unsafe { arch::write_frame(sp as *mut usize, entry as usize, arg) };
        sp
    }

    /// Save the running context's stack pointer to `*from` and resume the
    /// context saved at `to`. Returns when something switches back to
    /// `*from`.
    ///
    /// # Safety
    /// `to` must be a stack pointer saved by `switch` or made by `prepare`,
    /// whose stack is still mapped, and nothing else may resume it.
    pub unsafe fn switch(from: *mut usize, to: usize) {
        // SAFETY: upheld by the caller.
        unsafe { arch::switch(from, to) }
    }
}

#[cfg(not(all(
    any(target_os = "linux", target_os = "macos"),
    any(target_arch = "x86_64", target_arch = "aarch64")
)))]
mod platform {
    use std::io;

    pub const SUPPORTED: bool = false;

    #[derive(Clone, Copy, Debug)]
    pub struct PollFd {
        pub fd: i32,
        pub events: i16,
        pub revents: i16,
    }
    pub const POLLIN: i16 = 0x1;
    pub const POLLOUT: i16 = 0x4;

    pub struct Stack;
    impl Stack {
        pub fn new(_size: usize) -> io::Result<Stack> {
            Err(io::Error::other(
                "fibers are not supported on this platform",
            ))
        }
        pub fn top(&self) -> usize {
            0
        }
        pub fn size(&self) -> usize {
            0
        }
        pub fn release_pages(&self) {}
    }
    pub fn poll_fds(_fds: &mut [PollFd], _timeout: i32) -> io::Result<usize> {
        Err(io::Error::other("poll is not supported on this platform"))
    }
    pub fn prepare(_stack: &Stack, _entry: extern "C" fn(usize) -> !, _arg: usize) -> usize {
        0
    }
    pub unsafe fn switch(_from: *mut usize, _to: usize) {
        unreachable!("fibers are not supported on this platform")
    }
}

pub use self::platform::*;
