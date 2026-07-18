//! Ops-metric collection for a real eval run (task T014).
//!
//! Fills [`crate::score::OpsMetrics`] from primitive samples. Every metric is split into a pure,
//! unit-testable computation plus (at most) one thin IO sampler: the "injectable clock/reader" is
//! simply that `cpu_pct` takes explicit tick counts and an elapsed `Duration` — tests inject
//! values, the runner injects `/proc` reads and a real clock.

use std::io;
use std::path::Path;
use std::time::Duration;

/// Linux's near-universal `sysconf(_SC_CLK_TCK)` value.
// ponytail: hardcoded 100 instead of a libc call; `cpu_pct` takes it as a parameter anyway.
pub const DEFAULT_TICKS_PER_SEC: f64 = 100.0;

/// Observations stored per day over the capture window. Fails closed: a zero, negative, or
/// non-finite day span yields `0.0` rather than an infinite rate.
pub fn observations_per_day(count: u64, days: f64) -> f64 {
    if !days.is_finite() || days <= 0.0 {
        return 0.0;
    }
    count as f64 / days
}

/// Disk growth between two [`dir_size_bytes`] samples. A shrink reads as zero growth.
pub fn disk_growth_bytes(before: u64, after: u64) -> u64 {
    after.saturating_sub(before)
}

/// Total size in bytes of every regular file under `path`, recursively.
///
/// Symlinks are skipped (no cycle-chasing); any IO error propagates.
pub fn dir_size_bytes(path: &Path) -> io::Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            total += dir_size_bytes(&entry.path())?;
        } else if ft.is_file() {
            total += entry.metadata()?.len();
        }
        // symlinks: skipped
    }
    Ok(total)
}

/// Extract `utime + stime` (clock ticks) from a `/proc/<pid>/stat` line.
///
/// The comm field (field 2) may contain spaces and `)`, so parsing anchors on the *last* `)`;
/// after it, `utime` and `stime` are the 12th and 13th space-separated fields (0-indexed 11, 12).
/// Returns `None` on any malformed line — the caller treats that as "no sample".
pub fn parse_proc_stat_ticks(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    utime.checked_add(stime)
}

/// Read the current CPU tick count of `pid` from `/proc/<pid>/stat`.
///
/// One of the two samples for [`cpu_pct`]; pair it with an [`std::time::Instant`] taken at the
/// same moment.
pub fn read_pid_cpu_ticks(pid: u32) -> io::Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    parse_proc_stat_ticks(&stat)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unparseable /proc stat line"))
}

/// Average CPU percentage (of one core) between two tick samples over `elapsed` wall time.
///
/// Fails closed: zero elapsed time or a backwards tick delta yields `0.0`.
pub fn cpu_pct(start_ticks: u64, end_ticks: u64, elapsed: Duration, ticks_per_sec: f64) -> f64 {
    let secs = elapsed.as_secs_f64();
    if secs <= 0.0 || !ticks_per_sec.is_finite() || ticks_per_sec <= 0.0 || end_ticks < start_ticks
    {
        return 0.0;
    }
    let cpu_secs = (end_ticks - start_ticks) as f64 / ticks_per_sec;
    cpu_secs / secs * 100.0
}

/// Extraction precision over a hand-scored sample: the fraction judged correct.
///
/// Each entry is the judgment for one sampled (extraction, judgment) pair — the extraction text
/// itself doesn't enter the math, so callers pass judgments only. An empty sample fails closed
/// to `0.0` (no evidence ≠ perfect precision).
pub fn extraction_precision(judgments: &[bool]) -> f64 {
    if judgments.is_empty() {
        return 0.0;
    }
    judgments.iter().filter(|&&ok| ok).count() as f64 / judgments.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Duration;

    /// A unique scratch dir per test (std-only; no tempfile dep).
    fn test_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("homn-eval-ops-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn observations_per_day_divides_count_by_span() {
        assert_eq!(observations_per_day(700, 7.0), 100.0);
    }

    #[test]
    fn observations_per_day_fails_closed_on_bad_span() {
        assert_eq!(observations_per_day(700, 0.0), 0.0);
        assert_eq!(observations_per_day(700, -1.0), 0.0);
        assert_eq!(observations_per_day(700, f64::NAN), 0.0);
    }

    #[test]
    fn disk_growth_is_a_saturating_delta() {
        assert_eq!(disk_growth_bytes(1_000, 4_096), 3_096);
        assert_eq!(
            disk_growth_bytes(4_096, 1_000),
            0,
            "shrink reads as zero growth"
        );
    }

    #[test]
    fn dir_size_walks_nested_files() {
        let root = test_dir("walk");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.bin"), [0u8; 10]).unwrap();
        std::fs::write(root.join("sub").join("b.bin"), [0u8; 32]).unwrap();
        assert_eq!(dir_size_bytes(&root).unwrap(), 42);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn dir_size_of_missing_path_is_an_error() {
        assert!(dir_size_bytes(Path::new("/nonexistent/homn-eval-ops")).is_err());
    }

    #[test]
    fn parse_stat_reads_utime_plus_stime_past_the_comm_field() {
        // comm may contain spaces and ')' — parsing must anchor on the LAST ')'.
        let stat = "1234 (my (we)ird) exe) R 1 1234 1234 0 -1 4194560 100 0 0 0 70 30 0 0 20 0 1 0 100 0 0";
        assert_eq!(parse_proc_stat_ticks(stat), Some(100)); // utime 70 + stime 30
    }

    #[test]
    fn parse_stat_rejects_malformed_lines() {
        assert_eq!(parse_proc_stat_ticks(""), None);
        assert_eq!(
            parse_proc_stat_ticks("1234 (x) R 1"),
            None,
            "too few fields"
        );
        assert_eq!(parse_proc_stat_ticks("no comm parens at all"), None);
    }

    #[test]
    fn cpu_pct_from_tick_delta_and_wall_clock() {
        // 50 ticks over 1s at 100 ticks/s = 50% of one core.
        assert_eq!(cpu_pct(100, 150, Duration::from_secs(1), 100.0), 50.0);
    }

    #[test]
    fn cpu_pct_fails_closed_on_zero_elapsed_or_backwards_ticks() {
        assert_eq!(cpu_pct(100, 150, Duration::ZERO, 100.0), 0.0);
        assert_eq!(cpu_pct(150, 100, Duration::from_secs(1), 100.0), 0.0);
    }

    #[test]
    fn extraction_precision_is_fraction_judged_correct() {
        assert_eq!(extraction_precision(&[true, true, false, true]), 0.75);
    }

    #[test]
    fn extraction_precision_of_empty_sample_is_zero() {
        assert_eq!(extraction_precision(&[]), 0.0);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn read_own_pid_ticks_smoke() {
        // Our own /proc/<pid>/stat must parse.
        assert!(read_pid_cpu_ticks(std::process::id()).is_ok());
    }
}
