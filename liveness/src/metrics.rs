//! Guest resource metrics: pure parsers over the kernel interfaces.
//!
//! All collectors read only `/proc` and `statvfs`, so they need no external
//! commands and work from the earliest boot state onward.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use minimal_vm_liveness_protocol::SystemdState;
use minimal_vm_liveness_protocol::Usage;

/// CPU utilisation sampler: holds the previous `/proc/stat` snapshot so
/// [`sample`] can diff against it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CpuSample {
  first: ProcStat,
}

/// One `/proc/stat` `cpu` line: total and idle jiffies across all cores.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ProcStat {
  total: u64,
  idle: u64,
}

impl ProcStat {
  /// Parse the aggregate `cpu ` line (the one with a trailing space, before
  /// the per-core `cpu0`, `cpu1`, … lines).
  fn parse(line: &str) -> Option<Self> {
    let rest = line.strip_prefix("cpu ")?;
    let mut fields = rest.split_whitespace().map(|f| f.parse::<u64>().ok());
    // user nice system idle iowait irq softirq steal guest guest_nice
    let mut sum: u64 = 0;
    let mut idle: u64 = 0;
    for (i, field) in fields.by_ref().enumerate() {
      let value = match field {
        | Some(v) => v,
        // A truncated line still yields a usable total for the fields read.
        | None => break,
      };
      match i {
        // idle + iowait: time the CPU had nothing runnable.
        | 3 | 4 => idle += value,
        | _ => {},
      }
      sum = sum.checked_add(value)?;
    }
    Some(Self { total: sum, idle })
  }
}

/// Begin a CPU sample: snapshot the counters now; report with [`sample`]
/// after [`minimal_vm_liveness_protocol::INTERVAL`].
pub fn cpu_sample() -> Result<CpuSample> {
  Ok(CpuSample {
    first: read_proc_stat()?,
  })
}

/// Complete the CPU sample: utilisation since [`cpu_sample`] as a
/// percentage (0–100) across all cores. `Err` when `/proc/stat` became
/// unreadable or the counters did not advance (kernel restart, or the two
/// samples landed in the same jiffy).
pub fn sample(cpu: &mut CpuSample) -> Result<f64> {
  let second = read_proc_stat()?;
  let first = cpu.first;
  *cpu = CpuSample { first: second };
  cpu_percent(first, second).ok_or_else(|| anyhow::anyhow!("/proc/stat counters did not advance"))
}

/// Pure busy-jiffie ratio between two counter snapshots, as a percentage
/// clamped to 0–100. `None` when no time elapsed between the snapshots.
fn cpu_percent(first: ProcStat, second: ProcStat) -> Option<f64> {
  let total_delta = second.total.checked_sub(first.total)?;
  if total_delta == 0 {
    return None;
  }
  let busy_delta = (second.total - second.idle).checked_sub(first.total - first.idle)?;
  Some((busy_delta as f64 / total_delta as f64 * 100.0).clamp(0.0, 100.0))
}

fn read_proc_stat() -> Result<ProcStat> {
  let stat = std::fs::read_to_string("/proc/stat").context("reading /proc/stat")?;
  stat
    .lines()
    .find_map(ProcStat::parse)
    .ok_or_else(|| anyhow::anyhow!("/proc/stat has no aggregate cpu line"))
}

/// RAM used / total in bytes, from `/proc/meminfo`. `used` is the
/// classic `MemTotal − MemAvailable` (what the kernel could not give a new
/// allocation without reclaim).
pub fn memory() -> Result<Usage> {
  let meminfo = std::fs::read_to_string("/proc/meminfo").context("reading /proc/meminfo")?;
  let field = |prefix: &str| {
    meminfo.lines().find_map(|line| {
      line
        .strip_prefix(prefix)
        .and_then(|rest| rest.trim_end_matches(" kB").trim().parse::<u64>().ok())
    })
  };
  let total_kb = field("MemTotal:").context("/proc/meminfo has no MemTotal")?;
  let available_kb = field("MemAvailable:").context("/proc/meminfo has no MemAvailable")?;
  let used_kb = total_kb.saturating_sub(available_kb);
  Ok(Usage {
    used: used_kb * 1024,
    total: total_kb * 1024,
  })
}

/// Root filesystem used / total in bytes, via `statvfs`.
pub fn disk() -> Result<Usage> {
  disk_of("/")
}

fn disk_of(path: impl AsRef<Path>) -> Result<Usage> {
  let path = path.as_ref();
  let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
    .context("mount point contains a NUL byte")?;
  // SAFETY: `statvfs` is called with a valid NUL-terminated path; the
  // returned struct is fully initialized by the kernel on success.
  let stat = unsafe {
    let mut stat: libc::statvfs = std::mem::zeroed();
    let rc = libc::statvfs(c_path.as_ptr(), &mut stat);
    if rc != 0 {
      return Err(anyhow::anyhow!(
        "statvfs({}) failed: {}",
        path.display(),
        std::io::Error::last_os_error()
      ));
    }
    stat
  };
  let block = stat.f_frsize.max(stat.f_bsize) as u64;
  Ok(Usage {
    used: (stat.f_blocks.saturating_sub(stat.f_bfree)) * block,
    total: stat.f_blocks * block,
  })
}

/// systemd's overall system state, from `systemctl is-system-running`.
///
/// A failed `systemctl` (systemd not present/running yet) maps to
/// [`SystemdState::Unknown`] instead of failing the whole report: early-boot
/// reports must still reach the host.
pub fn systemd_state() -> SystemdState {
  match std::process::Command::new("systemctl")
    .arg("is-system-running")
    .output()
  {
    | Ok(output) => {
      let text = String::from_utf8_lossy(&output.stdout);
      SystemdState::parse(&text)
    },
    | Err(_) => SystemdState::Unknown,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn proc_stat_parses_aggregate_cpu_line() {
    let line = "cpu  100 0 200 1000 100 0 0 0 0 0";
    let stat = ProcStat::parse(line).expect("parse");
    // user 100 + nice 0 + system 200 + idle 1000 + iowait 100 = 1400 total,
    // idle = 1000 + 100 = 1100.
    assert_eq!(
      stat,
      ProcStat {
        total: 1400,
        idle: 1100
      }
    );
  }

  #[test]
  fn proc_stat_ignores_per_core_lines() {
    let text = "cpu0 1 2 3 4 5 6 7 8\n\
                cpu  10 20 30 400 0 0 0 0 0\n\
                cpu1 9 8 7 6 5 4 3 2\n";
    let stat = text.lines().find_map(ProcStat::parse).expect("parse");
    assert_eq!(
      stat,
      ProcStat {
        total: 460,
        idle: 400
      }
    );
  }

  #[test]
  fn proc_stat_rejects_non_cpu_lines() {
    assert_eq!(ProcStat::parse("intr 12345"), None);
    assert_eq!(ProcStat::parse("cpu0 1 2 3 4 5"), None);
  }

  #[test]
  fn cpu_percent_is_ratio_of_busy_jiffies() {
    // 200 jiffies elapse, 50 of them idle -> 75% busy.
    assert_eq!(
      cpu_percent(
        ProcStat {
          total: 1000,
          idle: 800
        },
        ProcStat {
          total: 1200,
          idle: 850
        }
      ),
      Some(75.0)
    );
    // No elapsed time (counters unchanged): no measurement is possible.
    assert_eq!(
      cpu_percent(
        ProcStat {
          total: 1000,
          idle: 800
        },
        ProcStat {
          total: 1000,
          idle: 800
        }
      ),
      None
    );
    // Idle-only interval: 0% busy, distinct from "unknown".
    assert_eq!(
      cpu_percent(
        ProcStat {
          total: 1000,
          idle: 800
        },
        ProcStat {
          total: 1100,
          idle: 900
        }
      ),
      Some(0.0)
    );
  }

  #[test]
  fn memory_reads_proc_meminfo() {
    // /proc/meminfo exists in any Linux container; just sanity-check the
    // invariants rather than exact values.
    let usage = memory().expect("memory");
    assert!(usage.total > 0);
    assert!(usage.used <= usage.total);
  }

  #[test]
  fn disk_reads_root_fs() {
    let usage = disk().expect("disk");
    assert!(usage.total > 0);
    assert!(usage.used <= usage.total);
  }

  #[test]
  fn meminfo_field_parser_handles_kib_suffix() {
    let meminfo = "MemTotal:       16088156 kB\nMemAvailable:   1000000 kB\n";
    let field = |prefix: &str| {
      meminfo.lines().find_map(|line| {
        line
          .strip_prefix(prefix)
          .and_then(|rest| rest.trim_end_matches(" kB").trim().parse::<u64>().ok())
      })
    };
    assert_eq!(field("MemTotal:"), Some(16088156));
    assert_eq!(field("MemAvailable:"), Some(1000000));
  }
}
