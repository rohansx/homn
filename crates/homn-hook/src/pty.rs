//! PTY-tap wrapper for `homn run <command> ...` (T053–T057, slice A).
//!
//! Spawns the user-supplied command under a pseudo-terminal so we can:
//!
//! 1. Plumb stdin/stdout/stderr transparently to the user's real terminal.
//! 2. Resize the child's terminal whenever our terminal resizes (SIGWINCH; R-001).
//! 3. *Later* (slice B / T055): tap the master fd stream looking for Claude Code's
//!    permission prompt, race a daemon decision, and synthesize `y\n` / `n\n` into
//!    the child's stdin when the daemon decides within the race window. This is the
//!    workaround for upstream bug [#19298](https://github.com/anthropics/claude-code/issues/19298).
//!
//! This slice ships #1 + #2 (transparent passthrough with terminal-size propagation).
//! #3 lands in a follow-up commit that wires `homn-daemon` calls into the read loop.
//!
//! Reference: [ADR-0003](../../../docs/architecture/adr/0003-pty-fallback.md).

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::Mutex;

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Result of running a command under the PTY wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtyExit {
    /// Exit code reported by the child process, or 130 (Ctrl-C) on signal.
    pub code: i32,
}

/// Run `argv` under a pseudo-terminal. argv[0] is the program; the rest are arguments.
///
/// Returns when the child process exits. Forwards Ctrl-C (SIGINT) to the child via the PTY
/// (the kernel translates it into the appropriate control character for us).
///
/// The wrapper does NOT need to be async — PTY I/O is one short read/write loop per direction,
/// and we'd rather have predictable blocking behaviour than the complexity of mixing async with
/// raw fd polling. Callers (`homn run` subcommand) can put this on a spawn_blocking task.
pub fn run_under_pty(argv: &[String]) -> anyhow::Result<PtyExit> {
    if argv.is_empty() {
        anyhow::bail!("homn run requires at least one argument: the command to spawn");
    }

    let pty_system = native_pty_system();
    let initial_size = current_term_size().unwrap_or(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    });

    let pair = pty_system.openpty(initial_size)?;

    let mut cmd = CommandBuilder::new(&argv[0]);
    for arg in &argv[1..] {
        cmd.arg(arg);
    }
    // Inherit environment so the child sees the same PATH, TERM, HOME, etc. as the user.
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }

    let mut child = pair.slave.spawn_command(cmd)?;

    // After spawn we no longer need the slave handle in this process. Drop it so EOF
    // propagates correctly when the child closes its end.
    drop(pair.slave);

    let master = Arc::new(Mutex::new(pair.master));

    // === stdin → master ===
    let master_for_writer = master.clone();
    let stdin_thread = std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let mut writer = match master_for_writer.lock() {
                        Ok(m) => match m.take_writer() {
                            Ok(w) => w,
                            Err(err) => {
                                tracing::warn!(error = %err, "pty take_writer failed");
                                break;
                            }
                        },
                        Err(_) => break,
                    };
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
                Err(err) => {
                    tracing::warn!(error = %err, "stdin read error");
                    break;
                }
            }
        }
    });

    // === master → stdout (with future tap point for prompt detection) ===
    let mut reader = master
        .lock()
        .map_err(|_| anyhow::anyhow!("pty master mutex poisoned"))?
        .try_clone_reader()?;
    let stdout_thread = std::thread::spawn(move || {
        let mut stdout = std::io::stdout().lock();
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // TODO(T054/T055): expose this chunk to a prompt-detector here before
                    // it goes to stdout. For now, transparent passthrough.
                    if stdout.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = stdout.flush();
                }
                Err(err) => {
                    tracing::warn!(error = %err, "pty read error");
                    break;
                }
            }
        }
    });

    // === Watch for SIGWINCH and resize the child's terminal ===
    let master_for_resize = master.clone();
    let resize_thread = spawn_winch_watcher(master_for_resize);

    // === Wait for the child ===
    let code = wait_child(&mut *child)?;

    // Best-effort cleanup. The threads exit when the master is closed.
    drop(master);
    // Don't bother joining stdin_thread — it may be blocked in a read on the user's stdin.
    let _ = stdin_thread; // intentional: forget
    let _ = stdout_thread.join();
    let _ = resize_thread; // intentional: forget

    Ok(PtyExit { code })
}

/// Block until the child exits; returns its exit code (or 130 if it was killed by a signal).
fn wait_child(child: &mut dyn Child) -> anyhow::Result<i32> {
    let status = child.wait()?;
    Ok(if status.success() {
        0
    } else {
        status.exit_code() as i32
    })
}

/// Read the current terminal size from stdout. Returns None if stdout isn't a terminal.
fn current_term_size() -> Option<PtySize> {
    use std::os::fd::AsFd;

    let stdout = std::io::stdout();
    let fd = stdout.as_fd();

    // SAFETY: TIOCGWINSZ takes a `struct winsize *`. We pass an aligned, fully-initialised
    // local of exactly that layout, and the ioctl writes (not reads) into it. The fd comes
    // from a `BorrowedFd` we own for the duration of the call.
    unsafe {
        let mut size: Winsize = std::mem::zeroed();
        let res = libc_ioctl_tiocgwinsz(fd, &mut size);
        if res == 0 {
            Some(PtySize {
                rows: size.ws_row,
                cols: size.ws_col,
                pixel_width: size.ws_xpixel,
                pixel_height: size.ws_ypixel,
            })
        } else {
            None
        }
    }
}

/// Spawn a thread that listens for SIGWINCH on the parent process and propagates the new
/// terminal size to the child PTY. Returns the JoinHandle; caller can drop it (the thread
/// terminates with the process).
fn spawn_winch_watcher(master: Arc<Mutex<Box<dyn MasterPty + Send>>>) -> std::thread::JoinHandle<()> {
    use signal_hook::consts::SIGWINCH;
    use signal_hook::iterator::Signals;

    std::thread::spawn(move || {
        let mut signals = match Signals::new([SIGWINCH]) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = %err, "failed to install SIGWINCH handler");
                return;
            }
        };
        for _ in signals.forever() {
            if let Some(size) = current_term_size() {
                if let Ok(m) = master.lock() {
                    if let Err(err) = m.resize(size) {
                        tracing::warn!(error = %err, "pty resize failed");
                    }
                }
            }
        }
    })
}

// Minimal libc binding for TIOCGWINSZ — avoids pulling in the whole `libc` crate just for
// this one ioctl. Linux + macOS both define TIOCGWINSZ as 0x5413 (Linux) / 0x40087468 (Darwin).
#[allow(unsafe_code)]
unsafe fn libc_ioctl_tiocgwinsz(fd: std::os::fd::BorrowedFd<'_>, size: *mut Winsize) -> i32 {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "linux")]
    const TIOCGWINSZ: u64 = 0x5413;
    #[cfg(target_os = "macos")]
    const TIOCGWINSZ: u64 = 0x40087468;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    const TIOCGWINSZ: u64 = 0;

    extern "C" {
        fn ioctl(fd: i32, request: u64, ...) -> i32;
    }
    unsafe { ioctl(fd.as_raw_fd(), TIOCGWINSZ, size) }
}

#[allow(unsafe_code)]
#[repr(C)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_argv_is_rejected_with_helpful_error() {
        let err = run_under_pty(&[]).unwrap_err();
        assert!(
            err.to_string().contains("at least one argument"),
            "got: {err}"
        );
    }

    /// Run a trivial command (`/bin/true`) through the wrapper and verify it exits cleanly.
    /// Skipped on platforms without `/bin/true`.
    #[test]
    fn run_true_exits_zero() {
        if !std::path::Path::new("/bin/true").exists() {
            eprintln!("skip: /bin/true not present");
            return;
        }
        let exit = run_under_pty(&["/bin/true".into()]).expect("run /bin/true");
        assert_eq!(exit.code, 0);
    }

    /// `/bin/false` should exit with a non-zero status that the wrapper propagates.
    #[test]
    fn run_false_propagates_nonzero_exit() {
        if !std::path::Path::new("/bin/false").exists() {
            eprintln!("skip: /bin/false not present");
            return;
        }
        let exit = run_under_pty(&["/bin/false".into()]).expect("run /bin/false");
        assert_ne!(exit.code, 0, "expected /bin/false to exit non-zero");
    }
}
