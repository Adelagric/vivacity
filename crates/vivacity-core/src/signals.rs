//! `require`'s signal handler (RequireCommand.php:160-162, seld
//! `SignalHandler`): from the moment the command has read its backups to
//! the end of the command, SIGINT, SIGTERM and SIGHUP revert composer.json
//! (and the lock) as `revertComposerFile` does, then the process dies by the
//! same signal (`exitWithLastSignal`: the default action restored, the
//! signal sent again; exit `128 + n` if it survives).
//!
//! The handler only marks the run as aborting and writes the signal number
//! to a pipe (async-signal-safe); a thread reads it and runs the revert.
//! Composer runs its handler on the interrupted main thread, so nothing of
//! the command runs after it: here every other thread stops at its next
//! output line ([`checkpoint`]) or write of composer.json or the lock
//! ([`write_gate`], held across the write: the revert waits for a write in
//! progress, and no write follows it).
//!
//! Composer's `InstallationManager::execute` installs its own handler while
//! the package operations run (InstallationManager.php:186-190), and seld
//! calls the most recent one only: no revert there, the process just dies
//! by the signal ([`suspend`]).
//!
//! Only for the binary ([`enable`]): an embedder of the library keeps its
//! own signal dispositions. macOS and Linux (`sigaction`, through the
//! `libc` this crate already uses there); elsewhere nothing is installed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ENABLED: AtomicBool = AtomicBool::new(false);
static ABORTING: AtomicBool = AtomicBool::new(false);
static SUSPENDED: AtomicBool = AtomicBool::new(false);
/// [`SUSPENDED`] when the signal came (read by the handling thread).
static SUSPENDED_AT_SIGNAL: AtomicBool = AtomicBool::new(false);
static HANDLING_THREAD: Mutex<Option<std::thread::ThreadId>> = Mutex::new(None);
/// Held across each write of composer.json or the lock; the handling
/// thread takes it for good before it reverts.
static WRITES: Mutex<()> = Mutex::new(());

/// The binary handles termination signals as Composer does (the library
/// alone installs nothing).
pub fn enable() {
    ENABLED.store(true, Ordering::Release);
}

/// While a signal is being handled, any other thread that reaches an output
/// line or a write of composer.json or the lock stops there for good: the
/// handling thread reverts the files, then ends the process. One atomic
/// load otherwise.
pub fn checkpoint() {
    if !ABORTING.load(Ordering::Acquire) || handling_thread() {
        return;
    }
    loop {
        std::thread::park();
    }
}

fn handling_thread() -> bool {
    HANDLING_THREAD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .is_some_and(|id| *id == std::thread::current().id())
}

/// Held while composer.json or the lock is written: a signal handled
/// meanwhile reverts once the write is done, and no write starts after it
/// (the thread stops here). Nothing for the handling thread itself.
pub struct WriteGate(#[allow(dead_code)] Option<std::sync::MutexGuard<'static, ()>>);

/// See [`WriteGate`].
pub fn write_gate() -> WriteGate {
    checkpoint();
    if ABORTING.load(Ordering::Acquire) && handling_thread() {
        return WriteGate(None);
    }
    WriteGate(Some(
        WRITES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    ))
}

/// The package operations run (`InstallationManager::execute`): a signal
/// ends the process without the revert.
pub fn suspend() {
    SUSPENDED.store(true, Ordering::Release);
}

/// The package operations are over: the revert applies again.
pub fn resume() {
    SUSPENDED.store(false, Ordering::Release);
}

/// Keeps the handlers while alive ([`on_termination`]); dropping it puts the
/// previous dispositions back.
#[must_use = "the handlers are removed when the guard is dropped"]
pub struct Guard(bool);

impl Drop for Guard {
    fn drop(&mut self) {
        // Only the guard that installed them removes them.
        if self.0 {
            release();
        }
    }
}

/// Installs the handlers, `revert` being what the handling thread runs
/// before the process dies by the signal. Nothing when the binary did not
/// [`enable`] it, or on a platform without `sigaction`.
pub fn on_termination(revert: impl FnOnce() + Send + 'static) -> Guard {
    Guard(ENABLED.load(Ordering::Acquire) && imp::register(Box::new(revert)))
}

/// Removes the handlers (the previous dispositions back): at the end of the
/// command, or before a child Composer takes the whole command over (it
/// installs its own).
pub fn release() {
    imp::unregister();
    SUSPENDED.store(false, Ordering::Release);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod imp {
    use super::{ABORTING, HANDLING_THREAD, SUSPENDED, SUSPENDED_AT_SIGNAL, WRITES};
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Mutex;

    const SIGNALS: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

    static WRITE_FD: AtomicI32 = AtomicI32::new(-1);
    /// The process that installed the handlers: a child between `fork` and
    /// `exec` inherits them and must not trigger the revert.
    static PID: AtomicI32 = AtomicI32::new(0);

    struct Registered {
        previous: Vec<(libc::c_int, libc::sigaction)>,
        read_fd: libc::c_int,
        write_fd: libc::c_int,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    // SAFETY: the saved `sigaction` structures are plain data, only used
    // again to restore the dispositions.
    unsafe impl Send for Registered {}

    static REGISTERED: Mutex<Option<Registered>> = Mutex::new(None);

    extern "C" fn on_signal(signal: libc::c_int) {
        // SAFETY: `getpid` is async-signal-safe.
        if unsafe { libc::getpid() } != PID.load(Ordering::Relaxed) {
            return;
        }
        let fd = WRITE_FD.load(Ordering::Relaxed);
        if fd >= 0 {
            // Before anything else of the run goes on (atomic stores are
            // async-signal-safe), as Composer's handler comes first.
            if !ABORTING.swap(true, Ordering::AcqRel) {
                SUSPENDED_AT_SIGNAL.store(SUSPENDED.load(Ordering::Acquire), Ordering::Release);
            }
            let byte = signal as u8;
            // SAFETY: `write` is async-signal-safe; one byte from the stack.
            unsafe {
                libc::write(fd, (&byte as *const u8).cast(), 1);
            }
        }
    }

    pub(super) fn register(revert: Box<dyn FnOnce() + Send>) -> bool {
        let mut slot = REGISTERED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_some() {
            return false;
        }
        let mut fds: [libc::c_int; 2] = [-1, -1];
        // SAFETY: a two-element buffer for `pipe`.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return false;
        }
        let (read_fd, write_fd) = (fds[0], fds[1]);
        for fd in fds {
            // Not inherited by the processes the command starts.
            // SAFETY: a descriptor this function just opened.
            unsafe {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
        }
        let thread = std::thread::Builder::new()
            .name("vivacity-signals".to_owned())
            .spawn(move || {
                *HANDLING_THREAD
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(std::thread::current().id());
                let mut byte = 0u8;
                loop {
                    // SAFETY: one byte into a stack buffer.
                    let n = unsafe { libc::read(read_fd, (&mut byte as *mut u8).cast(), 1) };
                    if n == 1 {
                        break;
                    }
                    if n < 0
                        && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                    {
                        continue;
                    }
                    return;
                }
                // 0: the handlers are being removed.
                if byte == 0 {
                    return;
                }
                let signal = libc::c_int::from(byte);
                ABORTING.store(true, Ordering::Release);
                // A write of composer.json or the lock in progress ends
                // first; none starts after (the gate is never given back).
                std::mem::forget(
                    WRITES
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
                if !SUSPENDED_AT_SIGNAL.load(Ordering::Acquire) {
                    // A panic in the revert still ends the process below.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(revert));
                }
                // `exitWithLastSignal`.
                // SAFETY: plain `sigaction`/`kill` calls on this process.
                unsafe {
                    let mut default: libc::sigaction = std::mem::zeroed();
                    default.sa_sigaction = libc::SIG_DFL;
                    libc::sigemptyset(&mut default.sa_mask);
                    libc::sigaction(signal, &default, std::ptr::null_mut());
                    libc::kill(libc::getpid(), signal);
                }
                std::process::exit(128 + signal);
            });
        let Ok(thread) = thread else {
            // SAFETY: closing the descriptors this function opened.
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
            return false;
        };
        // SAFETY: `getpid` has no preconditions.
        PID.store(unsafe { libc::getpid() }, Ordering::Release);
        WRITE_FD.store(write_fd, Ordering::Release);
        let mut previous = Vec::with_capacity(SIGNALS.len());
        for signal in SIGNALS {
            // SAFETY: a zeroed `sigaction` filled with a handler that only
            // writes to the pipe; the previous one kept for the restore.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
                libc::sigemptyset(&mut action.sa_mask);
                action.sa_flags = libc::SA_RESTART;
                let mut old: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(signal, &action, &mut old) == 0 {
                    previous.push((signal, old));
                }
            }
        }
        *slot = Some(Registered {
            previous,
            read_fd,
            write_fd,
            thread: Some(thread),
        });
        true
    }

    pub(super) fn unregister() {
        let Some(mut registered) = REGISTERED
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };
        for (signal, old) in &registered.previous {
            // SAFETY: the disposition saved when it was replaced.
            unsafe {
                libc::sigaction(*signal, old, std::ptr::null_mut());
            }
        }
        WRITE_FD.store(-1, Ordering::Release);
        let stop = 0u8;
        // SAFETY: one byte to the pipe this module opened.
        unsafe {
            libc::write(registered.write_fd, (&stop as *const u8).cast(), 1);
        }
        if let Some(thread) = registered.thread.take() {
            let _ = thread.join();
        }
        // SAFETY: the descriptors this module opened, closed once.
        unsafe {
            libc::close(registered.read_fd);
            libc::close(registered.write_fd);
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    pub(super) fn register(_revert: Box<dyn FnOnce() + Send>) -> bool {
        false
    }
    pub(super) fn unregister() {}
}
