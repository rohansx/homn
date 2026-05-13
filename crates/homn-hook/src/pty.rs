//! PTY-tap wrapper for `homn run <command> ...` (T053–T057).
//!
//! Spawns the user-supplied command under a pseudo-terminal so we can:
//!
//! 1. Plumb stdin/stdout/stderr transparently to the user's real terminal.
//! 2. Resize the child's terminal whenever our terminal resizes (SIGWINCH; R-001).
//! 3. Tap the master fd's output stream for Claude Code's permission prompt. When a
//!    prompt is detected, query the audit log for a recent `deny` (the hook subprocess
//!    has already fired and recorded the daemon's decision *before* Claude shows the
//!    prompt). If a deny is present, synthesize `n\n` into the PTY input — enforcing
//!    the policy despite upstream bug
//!    [#19298](https://github.com/anthropics/claude-code/issues/19298).
//!
//! Reference: [ADR-0003](../../../docs/architecture/adr/0003-pty-fallback.md).

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

/// Result of running a command under the PTY wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtyExit {
    /// Exit code reported by the child process.
    pub code: i32,
}

/// Configuration knobs for the PTY wrapper.
#[derive(Debug, Clone)]
pub struct PtyConfig {
    /// Regex (RE2 flavoured) that matches Claude Code's permission prompt in the PTY stream.
    pub prompt_regex: regex::Regex,
    /// Path to the daemon's audit SQLite database.
    pub audit_path: PathBuf,
    /// How far back (in seconds) to look for a recent `deny` when the prompt fires.
    pub deny_lookback_secs: u64,
    /// Master switch — set to `false` for transparent passthrough only.
    pub gating_enabled: bool,
}

impl PtyConfig {
    /// Build a transparent-passthrough config — useful for tests and `homn run` against
    /// non-Claude commands.
    pub fn passthrough() -> Self {
        Self {
            prompt_regex: regex::Regex::new(r"Do you want to proceed\? \(y/n\):").unwrap(),
            audit_path: PathBuf::from("/dev/null"),
            deny_lookback_secs: 5,
            gating_enabled: false,
        }
    }
}

/// Run `argv` under a pseudo-terminal. argv[0] is the program; the rest are arguments.
pub fn run_under_pty(argv: &[String], config: PtyConfig) -> anyhow::Result<PtyExit> {
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
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }

    let mut child = pair.slave.spawn_command(cmd)?;
    drop(pair.slave);

    let master = Arc::new(Mutex::new(pair.master));

    // === Centralised writer ===
    // Both the stdin-passthrough thread and the prompt-handler need to write to the
    // master fd. portable-pty's take_writer() can only be called once, so we centralise:
    // one thread holds the writer and pulls bytes off an mpsc channel; producers send into it.
    let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>();
    let writer_thread = spawn_writer_thread(master.clone(), write_rx)?;

    // === stdin → mpsc → master ===
    let stdin_tx = write_tx.clone();
    let stdin_thread = std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stdin_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // === master → stdout (+ prompt detection + gating) ===
    let reader = master
        .lock()
        .map_err(|_| anyhow::anyhow!("pty master mutex poisoned"))?
        .try_clone_reader()?;
    let gating_tx = write_tx.clone();
    let config_for_reader = config.clone();
    let stdout_thread =
        std::thread::spawn(move || master_reader_loop(reader, gating_tx, config_for_reader));

    // === SIGWINCH watcher ===
    let _resize_thread = spawn_winch_watcher(master.clone());

    // === Wait for child ===
    let code = wait_child(&mut *child)?;

    // Drop senders so the writer thread exits.
    drop(write_tx);
    drop(master); // closes the pty

    let _ = stdout_thread.join();
    let _ = writer_thread.join();
    // Don't join stdin_thread — it may be blocked on a read of the user's stdin.
    let _ = stdin_thread;

    Ok(PtyExit { code })
}

fn spawn_writer_thread(
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    rx: mpsc::Receiver<Vec<u8>>,
) -> anyhow::Result<std::thread::JoinHandle<()>> {
    // Take the writer once up front, then hand it to the thread.
    let mut writer = {
        let m = master
            .lock()
            .map_err(|_| anyhow::anyhow!("pty master poisoned"))?;
        m.take_writer()?
    };
    Ok(std::thread::spawn(move || {
        while let Ok(bytes) = rx.recv() {
            if writer.write_all(&bytes).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    }))
}

fn master_reader_loop(
    mut reader: Box<dyn Read + Send>,
    gating_tx: Sender<Vec<u8>>,
    config: PtyConfig,
) {
    let mut stdout = std::io::stdout().lock();
    let mut buf = [0u8; 8192];
    // Rolling window we check the prompt regex against. Large enough to span the
    // multi-line prompt Claude prints (~200 chars typical).
    let mut window: Vec<u8> = Vec::with_capacity(4096);
    const WINDOW_MAX: usize = 4096;

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                // Always passthrough first.
                if stdout.write_all(&buf[..n]).is_err() {
                    break;
                }
                let _ = stdout.flush();

                if !config.gating_enabled {
                    continue;
                }

                // Maintain the rolling window.
                window.extend_from_slice(&buf[..n]);
                if window.len() > WINDOW_MAX {
                    let drop = window.len() - WINDOW_MAX;
                    window.drain(..drop);
                }

                // Check for the prompt. Only the LAST match counts — we don't want to
                // re-fire on stale matches that already scrolled past.
                if let Ok(text) = std::str::from_utf8(&window) {
                    if let Some(m) = config.prompt_regex.find_iter(text).last() {
                        // De-dup: clear everything up through this match so we don't
                        // synthesize twice for the same prompt.
                        let end = m.end();
                        let cut = window.len().saturating_sub(text.len() - end);
                        window.drain(..cut);

                        let synth = decide_synth(&config);
                        if !synth.is_empty() {
                            tracing::info!(
                                synth = %String::from_utf8_lossy(&synth).trim(),
                                "homn run: synthesizing response to claude prompt"
                            );
                            if gating_tx.send(synth).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "pty read error");
                break;
            }
        }
    }
}

/// Decide what (if anything) to write into the PTY in response to a detected prompt.
/// Returns the bytes to send, or empty vec for "let the user decide".
fn decide_synth(config: &PtyConfig) -> Vec<u8> {
    match homn_audit::has_recent_deny_sync(&config.audit_path, config.deny_lookback_secs) {
        Ok(true) => b"n\n".to_vec(),
        Ok(false) => Vec::new(),
        Err(err) => {
            tracing::warn!(error = %err, "audit query failed; not synthesizing");
            Vec::new()
        }
    }
}

fn wait_child(child: &mut dyn Child) -> anyhow::Result<i32> {
    let status = child.wait()?;
    Ok(if status.success() {
        0
    } else {
        status.exit_code() as i32
    })
}

fn current_term_size() -> Option<PtySize> {
    use std::os::fd::AsFd;

    let stdout = std::io::stdout();
    let fd = stdout.as_fd();

    // SAFETY: TIOCGWINSZ writes a `struct winsize` into a pointer we own; the fd is valid
    // for the duration of the call.
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

fn spawn_winch_watcher(
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
) -> std::thread::JoinHandle<()> {
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
        let err = run_under_pty(&[], PtyConfig::passthrough()).unwrap_err();
        assert!(
            err.to_string().contains("at least one argument"),
            "got: {err}"
        );
    }

    #[test]
    fn run_true_exits_zero() {
        if !std::path::Path::new("/bin/true").exists() {
            eprintln!("skip: /bin/true not present");
            return;
        }
        let exit =
            run_under_pty(&["/bin/true".into()], PtyConfig::passthrough()).expect("run /bin/true");
        assert_eq!(exit.code, 0);
    }

    #[test]
    fn run_false_propagates_nonzero_exit() {
        if !std::path::Path::new("/bin/false").exists() {
            eprintln!("skip: /bin/false not present");
            return;
        }
        let exit = run_under_pty(&["/bin/false".into()], PtyConfig::passthrough())
            .expect("run /bin/false");
        assert_ne!(exit.code, 0);
    }

    #[test]
    fn passthrough_config_defaults_are_sensible() {
        let cfg = PtyConfig::passthrough();
        assert!(!cfg.gating_enabled);
        assert!(cfg.prompt_regex.is_match("Do you want to proceed? (y/n):"));
        assert!(!cfg.prompt_regex.is_match("hello world"));
    }
}
