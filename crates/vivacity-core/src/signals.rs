//! Termination signals as Composer handles them (docs/plans/parity-gaps-026.md,
//! 32b; the `unsafe` is justified in DECISIONS, 2026-10-30).
//!
//! - **`require`** (RequireCommand.php:160-164, seld `SignalHandler`): from
//!   the moment the command has read its backups to the end of the command,
//!   SIGINT, SIGTERM and SIGHUP revert composer.json (and the lock) as
//!   `revertComposerFile` does, then the process dies by the same signal
//!   (`exitWithLastSignal`: the default action restored, the signal sent
//!   again; exit `128 + n` without `posix`). [`on_termination`].
//! - **A hand-over** (the command re-run by a child Composer): while vivacity
//!   starts and waits for it, the three signals are passed on to the child,
//!   which handles them as Composer does; then vivacity ends as the child
//!   did — by the same signal when one ended it. [`spawn_forwarding`].
//!
//! One pipe and one thread for the process, created the first time either
//! is needed and kept until exit (never closed: a handler that loaded the
//! descriptor just before the handlers were removed never writes to a
//! reused one). The handler itself only saves `errno`, compares `getpid`,
//! loads and stores atomics and writes one byte from its stack to the
//! non-blocking write end; the thread does the rest. Composer runs its
//! handler on the interrupted main thread, so nothing of the command runs
//! after it: here every other thread stops at its next output line
//! ([`checkpoint`]) or write of composer.json or the lock ([`write_gate`],
//! held across the write — the revert waits for a write in progress, and no
//! write follows it). Every write of composer.json or composer.lock between
//! the backup and the end of `require` goes through the gate (the edit, the
//! lock, `updateRequirementsAfterResolution`, the revert, Flex's recipes);
//! after waiting [`GATE_WAIT`] for it, the revert goes on without it.
//!
//! Composer's `InstallationManager::execute` installs its own handler while
//! the package operations run (InstallationManager.php:186-190), and seld
//! calls the most recent one only: no revert there, the process dies by the
//! signal ([`suspend`]). Its `runCleanup` (partial downloads and
//! extractions removed) is not reproduced: what vivacity was extracting is
//! left as it was (inventoried).
//!
//! Only for the binary ([`enable`]): an embedder of the library keeps its
//! own signal dispositions. macOS and Linux (`sigaction`, through the `libc`
//! this crate already uses there); elsewhere nothing is installed.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::sync::Mutex;

static ENABLED: AtomicBool = AtomicBool::new(false);
static ABORTING: AtomicBool = AtomicBool::new(false);
static SUSPENDED: AtomicBool = AtomicBool::new(false);
/// [`SUSPENDED`] when the signal came (read by the handling thread).
static SUSPENDED_AT_SIGNAL: AtomicBool = AtomicBool::new(false);
/// The signal being handled under [`MODE_REVERT`].
static LAST_SIGNAL: AtomicI32 = AtomicI32::new(0);
static HANDLING_THREAD: Mutex<Option<std::thread::ThreadId>> = Mutex::new(None);
/// Held across each write of composer.json or the lock; the handling
/// thread takes it before it reverts and never gives it back.
static WRITES: Mutex<()> = Mutex::new(());
/// `require`'s revert.
#[allow(clippy::type_complexity)]
static REVERT: Mutex<Option<Box<dyn FnOnce() + Send>>> = Mutex::new(None);
/// Whether the process then dies by the signal (`posix_kill`), or exits
/// `128 + n` (`exitWithLastSignal` without `posix`).
static RAISES: Mutex<Option<fn() -> bool>> = Mutex::new(None);
/// A signal that came while the child was being started, passed on once
/// its pid is known.
static PENDING: AtomicI32 = AtomicI32::new(0);

const MODE_NONE: u8 = 0;
const MODE_REVERT: u8 = 1;
const MODE_FORWARD: u8 = 2;
/// What a signal does now: nothing of ours, `require`'s revert, or passed
/// on to the child Composer ([`CHILD`]).
static MODE: AtomicU8 = AtomicU8::new(MODE_NONE);
static CHILD: AtomicI32 = AtomicI32::new(0);

/// How long the handling thread waits for a write of composer.json or the
/// lock in progress (one file write: milliseconds) before it reverts
/// anyway: a thread that would hold the gate while blocked elsewhere must
/// not keep the process alive.
const GATE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

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
/// (the thread stops here). Nothing for the handling thread itself. Never
/// held across an output line (a thread stopped at its [`checkpoint`] would
/// hold it).
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

/// Keeps `require`'s handlers while alive ([`on_termination`]); dropping it
/// puts the previous dispositions back.
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

/// Installs `require`'s handlers, `revert` being what the handling thread
/// runs before the process ends by the signal — by the signal itself when
/// `raises` says so, `exit(128 + n)` otherwise (also during the package
/// operations, where no revert runs). Nothing when the binary did not
/// [`enable`] it, when handlers are already installed, or on a platform
/// without `sigaction`.
pub fn on_termination(revert: impl FnOnce() + Send + 'static, raises: fn() -> bool) -> Guard {
    if !ENABLED.load(Ordering::Acquire) || MODE.load(Ordering::Acquire) != MODE_NONE {
        return Guard(false);
    }
    *REVERT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Box::new(revert));
    *RAISES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(raises);
    if imp::install(MODE_REVERT) {
        Guard(true)
    } else {
        REVERT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Guard(false)
    }
}

/// Removes `require`'s handlers (the previous dispositions back): at the end
/// of the command, or before a child Composer takes the whole command over
/// (it installs its own).
pub fn release() {
    if MODE.load(Ordering::Acquire) == MODE_REVERT {
        imp::uninstall();
        // A signal caught before the removal is still handled (the thread
        // reads its byte after the mode changed): the revert stays for it.
        if !ABORTING.load(Ordering::Acquire) {
            REVERT
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
        }
    }
    SUSPENDED.store(false, Ordering::Release);
}

/// Starts the child Composer of a hand-over and waits for it, passing
/// SIGINT, SIGTERM and SIGHUP on to it from before it starts to after it
/// ends: a signal sent to vivacity alone (`timeout`, `docker stop`,
/// systemd's `KillMode=process`) reaches the Composer that runs the
/// command, which reverts and ends as it would run directly (one that came
/// while it was being started is passed on once its pid is known). A signal
/// the terminal sent to the whole process group reaches the child twice:
/// Composer's handlers and the default action end it on the first, so the
/// second changes nothing it prints or writes. The child is waited for
/// without being reaped until the handlers are removed (no signal goes to a
/// reused pid). Plain spawn and wait for the library or without
/// `sigaction`.
pub fn spawn_forwarding(
    cmd: &mut std::process::Command,
) -> std::io::Result<std::process::ExitStatus> {
    let forwarding =
        ENABLED.load(Ordering::Acquire) && MODE.load(Ordering::Acquire) == MODE_NONE && {
            CHILD.store(0, Ordering::Release);
            PENDING.store(0, Ordering::Release);
            imp::install(MODE_FORWARD)
        };
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            if forwarding {
                imp::uninstall();
            }
            return Err(e);
        }
    };
    if forwarding {
        if let Ok(pid) = i32::try_from(child.id()) {
            CHILD.store(pid, Ordering::Release);
            imp::forward_pending(pid);
            imp::wait_unreaped(pid);
        }
        imp::uninstall();
    }
    child.wait()
}

/// The exit code of a child Composer, or — for the binary, when a signal
/// ended it — this process ended by the same signal, as Composer run
/// directly would have ended (`128 + n` for the library, or if the signal
/// does not end it).
pub fn exit_code_of(status: std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    match imp::signal_of(status) {
        // Not a signal that dumps core: vivacity's own core is not
        // Composer's.
        Some(signal) if ENABLED.load(Ordering::Acquire) && !imp::dumps_core(signal) => {
            imp::die(signal, true)
        }
        Some(signal) => 128 + signal,
        None => 1,
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod imp {
    use super::{
        ABORTING, CHILD, GATE_WAIT, HANDLING_THREAD, LAST_SIGNAL, MODE, MODE_FORWARD, MODE_NONE,
        MODE_REVERT, PENDING, RAISES, REVERT, SUSPENDED, SUSPENDED_AT_SIGNAL, WRITES,
    };
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Mutex;

    const SIGNALS: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

    /// The pipe's write end, `-1` until it exists; never closed.
    static WRITE_FD: AtomicI32 = AtomicI32::new(-1);
    /// The process that installed the handlers: a child between `fork` and
    /// `exec` inherits them and must not act for the parent.
    static PID: AtomicI32 = AtomicI32::new(0);
    /// The dispositions the installed handlers replaced.
    static PREVIOUS: Mutex<Vec<(libc::c_int, libc::sigaction)>> = Mutex::new(Vec::new());
    static PIPE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

    #[cfg(target_os = "macos")]
    fn errno_location() -> *mut libc::c_int {
        // SAFETY: the calling thread's `errno`, always valid.
        unsafe { libc::__error() }
    }

    #[cfg(target_os = "linux")]
    fn errno_location() -> *mut libc::c_int {
        // SAFETY: the calling thread's `errno`, always valid.
        unsafe { libc::__errno_location() }
    }

    extern "C" fn on_signal(signal: libc::c_int) {
        // `write` may set `errno`: the interrupted code finds it unchanged.
        // SAFETY: the thread's own `errno`; `getpid` and `write` are
        // async-signal-safe, the byte is on this stack, the descriptor is
        // never closed and does not block.
        unsafe {
            let saved = *errno_location();
            if libc::getpid() == PID.load(Ordering::Relaxed) {
                let mode = MODE.load(Ordering::Acquire);
                let fd = WRITE_FD.load(Ordering::Acquire);
                if mode != MODE_NONE && fd >= 0 {
                    // Before anything else of the run goes on, as Composer's
                    // handler comes first.
                    if mode == MODE_REVERT && !ABORTING.swap(true, Ordering::AcqRel) {
                        SUSPENDED_AT_SIGNAL
                            .store(SUSPENDED.load(Ordering::Acquire), Ordering::Release);
                        LAST_SIGNAL.store(signal, Ordering::Release);
                    }
                    let byte = signal as u8;
                    libc::write(fd, (&byte as *const u8).cast(), 1);
                }
            }
            *errno_location() = saved;
        }
    }

    /// `pipe2(O_CLOEXEC)` on Linux; on macOS, which has no `pipe2`, `pipe`
    /// then `FD_CLOEXEC` (a process another thread spawns in between may
    /// inherit the two descriptors: it never writes to them).
    fn open_pipe() -> Option<(libc::c_int, libc::c_int)> {
        let mut fds: [libc::c_int; 2] = [-1, -1];
        #[cfg(target_os = "linux")]
        // SAFETY: a two-element buffer for `pipe2`.
        let opened = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } == 0;
        #[cfg(target_os = "macos")]
        // SAFETY: a two-element buffer for `pipe`, then the flag set on the
        // descriptors it just opened.
        let opened = unsafe {
            libc::pipe(fds.as_mut_ptr()) == 0
                && fds
                    .iter()
                    .all(|fd| libc::fcntl(*fd, libc::F_SETFD, libc::FD_CLOEXEC) == 0)
        };
        if !opened {
            return None;
        }
        // The handler never blocks on a full pipe (a lost byte is a signal
        // already pending).
        // SAFETY: the write end this function just opened.
        unsafe {
            let flags = libc::fcntl(fds[1], libc::F_GETFL);
            libc::fcntl(fds[1], libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        Some((fds[0], fds[1]))
    }

    /// The pipe and the thread that reads it, once for the process.
    fn ensure_pipe() -> bool {
        *PIPE.get_or_init(|| {
            let Some((read_fd, write_fd)) = open_pipe() else {
                return false;
            };
            let thread = std::thread::Builder::new()
                .name("vivacity-signals".to_owned())
                .spawn(move || dispatch(read_fd));
            if thread.is_err() {
                return false;
            }
            WRITE_FD.store(write_fd, Ordering::Release);
            true
        })
    }

    fn dispatch(read_fd: libc::c_int) {
        *HANDLING_THREAD
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(std::thread::current().id());
        let mut byte = 0u8;
        loop {
            // SAFETY: one byte into a stack buffer, from the read end this
            // module owns.
            let n = unsafe { libc::read(read_fd, (&mut byte as *mut u8).cast(), 1) };
            if n == 1 {
                let signal = libc::c_int::from(byte);
                // `ABORTING` is set by the handler under `require`'s mode
                // only: its signal is handled even if the handlers were
                // removed since (`release` keeps the revert then).
                if ABORTING.load(Ordering::Acquire) {
                    abort(LAST_SIGNAL.load(Ordering::Acquire));
                }
                if MODE.load(Ordering::Acquire) == MODE_FORWARD {
                    match CHILD.load(Ordering::Acquire) {
                        // Not started yet: passed on once it is.
                        0 => PENDING.store(signal, Ordering::Release),
                        // SAFETY: the child this process started and has
                        // not reaped (`wait_unreaped`).
                        pid => unsafe {
                            libc::kill(pid, signal);
                        },
                    }
                }
                // Otherwise a byte left from handlers already removed.
                continue;
            }
            if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            // The pipe cannot be read any more (defensive: the write end is
            // never closed): a signal already caught still ends the process.
            if ABORTING.load(Ordering::Acquire) {
                die(LAST_SIGNAL.load(Ordering::Acquire), true);
            }
            return;
        }
    }

    /// `require`'s handler: the revert, then the end by the signal.
    fn abort(signal: libc::c_int) -> ! {
        ABORTING.store(true, Ordering::Release);
        // A write of composer.json or the lock in progress ends first; none
        // starts after (the gate is never given back) — waited for
        // [`GATE_WAIT`] at most.
        let deadline = std::time::Instant::now() + GATE_WAIT;
        loop {
            match WRITES.try_lock() {
                Ok(gate) => {
                    std::mem::forget(gate);
                    break;
                }
                Err(std::sync::TryLockError::Poisoned(p)) => {
                    std::mem::forget(p.into_inner());
                    break;
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
        if !SUSPENDED_AT_SIGNAL.load(Ordering::Acquire) {
            let revert = REVERT
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(revert) = revert {
                // A panic in the revert still ends the process below.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(revert));
            }
        }
        let raises = RAISES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .unwrap_or(|| true);
        die(signal, std::panic::catch_unwind(raises).unwrap_or(true))
    }

    /// `exitWithLastSignal`: the default action back and the signal sent
    /// again (`raise`), then `exit(128 + n)` if the process is still there.
    /// `raise(3)` rather than `kill(getpid())`: the signal goes to this
    /// thread, not to another that might never run again.
    pub(super) fn die(signal: libc::c_int, raise: bool) -> ! {
        if raise {
            // SAFETY: plain `sigaction`/`raise` calls on this process.
            unsafe {
                let mut default: libc::sigaction = std::mem::zeroed();
                default.sa_sigaction = libc::SIG_DFL;
                libc::sigemptyset(&mut default.sa_mask);
                libc::sigaction(signal, &default, std::ptr::null_mut());
                libc::raise(signal);
            }
        }
        std::process::exit(128 + signal)
    }

    pub(super) fn dumps_core(signal: libc::c_int) -> bool {
        [
            libc::SIGQUIT,
            libc::SIGILL,
            libc::SIGTRAP,
            libc::SIGABRT,
            libc::SIGBUS,
            libc::SIGFPE,
            libc::SIGSEGV,
            libc::SIGSYS,
        ]
        .contains(&signal)
    }

    /// A signal that came while the child was being started.
    pub(super) fn forward_pending(pid: libc::c_int) {
        let signal = PENDING.swap(0, Ordering::AcqRel);
        if signal != 0 {
            // SAFETY: the child this process just started.
            unsafe {
                libc::kill(pid, signal);
            }
        }
    }

    /// Until the child has ended, without reaping it (`WNOWAIT`): its pid
    /// stays its own while signals may still be passed on to it.
    pub(super) fn wait_unreaped(pid: libc::c_int) {
        let Ok(id) = libc::id_t::try_from(pid) else {
            return;
        };
        loop {
            // SAFETY: a zeroed `siginfo_t` for `waitid` to fill.
            let rc = unsafe {
                let mut info: libc::siginfo_t = std::mem::zeroed();
                libc::waitid(libc::P_PID, id, &mut info, libc::WEXITED | libc::WNOWAIT)
            };
            if rc == 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
            {
                return;
            }
        }
    }

    pub(super) fn signal_of(status: std::process::ExitStatus) -> Option<libc::c_int> {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    }

    pub(super) fn install(mode: u8) -> bool {
        if !ensure_pipe() {
            return false;
        }
        let mut previous = PREVIOUS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !previous.is_empty() {
            return false;
        }
        // SAFETY: `getpid` has no preconditions.
        PID.store(unsafe { libc::getpid() }, Ordering::Release);
        MODE.store(mode, Ordering::Release);
        for signal in SIGNALS {
            // SAFETY: a zeroed `sigaction` filled with the handler above;
            // the previous one kept for the restore.
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
        true
    }

    pub(super) fn uninstall() {
        let mut previous = PREVIOUS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (signal, old) in previous.drain(..) {
            // SAFETY: the disposition saved when it was replaced.
            unsafe {
                libc::sigaction(signal, &old, std::ptr::null_mut());
            }
        }
        MODE.store(MODE_NONE, Ordering::Release);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    pub(super) fn install(_mode: u8) -> bool {
        false
    }
    pub(super) fn uninstall() {}
    pub(super) fn die(signal: i32, _raise: bool) -> ! {
        std::process::exit(128 + signal)
    }
    pub(super) fn signal_of(_status: std::process::ExitStatus) -> Option<i32> {
        None
    }
    pub(super) fn dumps_core(_signal: i32) -> bool {
        true
    }
    pub(super) fn forward_pending(_pid: i32) {}
    pub(super) fn wait_unreaped(_pid: i32) {}
}
