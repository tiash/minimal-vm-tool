//! Guest resource metrics: pure parsers over the kernel interfaces.
//!
//! All collectors read only `/proc` and `statvfs`, so they need no external
//! commands and work from the earliest boot state onward.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use minimal_vm_liveness_protocol::FileSystemUsage;
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
/// percentage (0–100) across all cores. A window in which no jiffies
/// elapsed (the first report is sent immediately after [`cpu_sample`])
/// reads as 0% — there was nothing measurable yet, and the next interval's
/// report carries the real delta. `Err` only when `/proc/stat` became
/// unreadable.
pub fn sample(cpu: &mut CpuSample) -> Result<f64> {
  let second = read_proc_stat()?;
  let first = cpu.first;
  *cpu = CpuSample { first: second };
  Ok(cpu_percent(first, second))
}

/// Pure busy-jiffie ratio between two counter snapshots, as a percentage
/// clamped to 0–100. Counters that run backwards (kernel restart mid-sample)
/// saturate to 0 rather than producing a bogus number.
fn cpu_percent(first: ProcStat, second: ProcStat) -> f64 {
  let total_delta = second.total.saturating_sub(first.total);
  if total_delta == 0 {
    return 0.0;
  }
  let busy_delta = (second.total - second.idle).saturating_sub(first.total - first.idle);
  (busy_delta as f64 / total_delta as f64 * 100.0).clamp(0.0, 100.0)
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
  let meminfo = read_meminfo()?;
  let total_kb = meminfo_field(&meminfo, "MemTotal:")?;
  let available_kb = meminfo_field(&meminfo, "MemAvailable:")?;
  let used_kb = total_kb.saturating_sub(available_kb);
  Ok(Usage {
    used: used_kb * 1024,
    total: total_kb * 1024,
  })
}

/// Swap used / total in bytes, from `/proc/meminfo`. A guest without swap
/// reports zeroes, which is a real measurement, not a failure.
pub fn swap() -> Result<Usage> {
  let meminfo = read_meminfo()?;
  let total_kb = meminfo_field(&meminfo, "SwapTotal:")?;
  let free_kb = meminfo_field(&meminfo, "SwapFree:")?;
  let used_kb = total_kb.saturating_sub(free_kb);
  Ok(Usage {
    used: used_kb * 1024,
    total: total_kb * 1024,
  })
}

fn read_meminfo() -> Result<String> {
  std::fs::read_to_string("/proc/meminfo").context("reading /proc/meminfo")
}

/// One `Name:  value kB` field of `/proc/meminfo`, as kilobytes.
fn meminfo_field(meminfo: &str, prefix: &str) -> Result<u64> {
  meminfo
    .lines()
    .find_map(|line| {
      line
        .strip_prefix(prefix)
        .and_then(|rest| rest.trim_end_matches(" kB").trim().parse::<u64>().ok())
    })
    .with_context(|| format!("/proc/meminfo has no {prefix}"))
}

/// Every writable guest filesystem: used / total in bytes per mount point,
/// via `statvfs` over the entries of `/proc/mounts`.
///
/// Skipped: 9p mounts (host-shared directories — their usage is the
/// host's, not the guest's), read-only mounts, `/run`, `/dev` and `/nix`
/// with their sub-mounts (runtime state, device nodes, and the nix store
/// — not storage the operator plans around), and pseudo-filesystems whose
/// `statvfs` reports no blocks (`proc`, `sysfs`, `devpts`, …).
///
/// Bind mounts of one filesystem (`/workspace` and `/tmp` both on
/// `/scratch`, say) share their device id, so they are reported once,
/// under the first mount point `/proc/mounts` lists for that device —
/// the original mount, with the binds created later appearing after it.
pub fn disks() -> Result<Vec<FileSystemUsage>> {
  let mounts = std::fs::read_to_string("/proc/mounts").context("reading /proc/mounts")?;
  let mut seen: Vec<String> = Vec::new();
  let mut devices: Vec<u64> = Vec::new();
  let mut disks = Vec::new();
  for line in mounts.lines() {
    let mut fields = line.split_whitespace();
    let (_device, mount_point, fstype, options) =
      match (fields.next(), fields.next(), fields.next(), fields.next()) {
        | (Some(d), Some(m), Some(t), Some(o)) => (d, m, t, o),
        // A truncated line has no mount to report.
        | _ => continue,
      };
    if fstype.starts_with("9p") {
      continue;
    }
    if options.split(',').any(|option| option == "ro") {
      continue;
    }
    let mount_point = unescape_mount_point(mount_point);
    if is_runtime_mount(&mount_point) {
      continue;
    }
    if seen.iter().any(|seen| seen == &mount_point) {
      continue;
    }
    seen.push(mount_point.clone());
    // A pseudo-filesystem (`proc`, `sysfs`, …) has no blocks to report;
    // an inaccessible mount point is equally not a usage sample.
    let usage = match disk_of(&mount_point) {
      | Ok(usage) if usage.total > 0 => usage,
      | _ => continue,
    };
    // The device id identifies the underlying filesystem: a bind mount
    // of an already-reported filesystem adds no new gauge.
    let device = std::fs::metadata(&mount_point)
      .map(|meta| std::os::unix::fs::MetadataExt::dev(&meta))
      .unwrap_or(u64::MAX);
    if devices.contains(&device) {
      continue;
    }
    devices.push(device);
    let label = device_label(device);
    disks.push(FileSystemUsage {
      mount_point,
      label,
      usage,
    });
  }
  Ok(disks)
}

/// The label udev gives the underlying device: the filesystem label
/// (`/dev/disk/by-label/<name>`), or else the virtio serial
/// (`/dev/disk/by-id/virtio-<serial>`). `None` when neither exists.
fn device_label(device: u64) -> Option<String> {
  let by_dir = |dir: &str| {
    std::fs::read_dir(dir)
      .ok()?
      .filter_map(|entry| entry.ok())
      .find_map(|entry| {
        let path = entry.path();
        let target = std::fs::metadata(&path).ok()?;
        if std::os::unix::fs::MetadataExt::dev(&target) != device {
          return None;
        }
        path.file_name()?.to_str().map(str::to_string)
      })
  };
  by_dir("/dev/disk/by-label").or_else(|| by_dir("/dev/disk/by-id"))
}

/// Whether a mount point is `/run`, `/dev` or `/nix` or inside either:
/// runtime state, device nodes, and the nix store — not filesystems
/// worth a gauge.
fn is_runtime_mount(mount_point: &str) -> bool {
  ["/run", "/dev", "/nix"]
    .iter()
    .any(|prefix| mount_point == *prefix || mount_point.starts_with(&format!("{prefix}/")))
}

/// Decode the octal escapes of `/proc/mounts` (`\040` for a space, `\011`
/// for a tab, `\134` for a backslash): a mount point containing them would
/// otherwise be statvfs'd at the wrong path.
fn unescape_mount_point(field: &str) -> String {
  let mut decoded = String::with_capacity(field.len());
  let mut chars = field.chars();
  while let Some(c) = chars.next() {
    if c != '\\' {
      decoded.push(c);
      continue;
    }
    let rest: String = chars.by_ref().take(3).collect();
    match u32::from_str_radix(&rest, 8) {
      | Ok(byte) if rest.len() == 3 => {
        decoded.push(char::from_u32(byte).unwrap_or(char::REPLACEMENT_CHARACTER))
      },
      | _ => {
        decoded.push('\\');
        decoded.push_str(&rest);
      },
    }
  }
  decoded
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
      75.0
    );
    // No elapsed time (the immediate first report): a valid 0% sample.
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
      0.0
    );
    // Idle-only interval: 0% busy.
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
      0.0
    );
    // Counters running backwards (kernel restart mid-sample) saturate to 0.
    assert_eq!(
      cpu_percent(
        ProcStat {
          total: 1000,
          idle: 800
        },
        ProcStat { total: 5, idle: 4 }
      ),
      0.0
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
  fn swap_reads_proc_meminfo() {
    // /proc/meminfo exists in any Linux container; just sanity-check the
    // invariants rather than exact values.
    let usage = swap().expect("swap");
    assert!(usage.used <= usage.total);
  }

  #[test]
  fn disks_report_every_writable_fs() {
    // /proc/mounts and statvfs exist in any Linux container; just
    // sanity-check the invariants rather than exact values.
    let disks = disks().expect("disks");
    assert!(
      disks.iter().any(|d| d.mount_point == "/"),
      "the root filesystem must be reported"
    );
    for disk in &disks {
      assert!(
        !is_runtime_mount(&disk.mount_point),
        "runtime mounts must not be reported: {}",
        disk.mount_point
      );
      assert!(disk.usage.total > 0);
      assert!(disk.usage.used <= disk.usage.total);
    }
    // One entry per underlying filesystem: no two reported mounts may
    // share a device id (bind mounts of one filesystem collapse).
    let mut devices: Vec<u64> = Vec::new();
    for disk in &disks {
      let device = std::fs::metadata(&disk.mount_point)
        .map(|meta| std::os::unix::fs::MetadataExt::dev(&meta))
        .expect("the mount point is statvfs-able, so it is stat-able");
      assert!(
        !devices.contains(&device),
        "one filesystem must be reported once: {}",
        disk.mount_point
      );
      devices.push(device);
    }
  }

  #[test]
  fn meminfo_field_parser_handles_kib_suffix() {
    let meminfo = "MemTotal:       16088156 kB\nMemAvailable:   1000000 kB\n";
    assert_eq!(meminfo_field(meminfo, "MemTotal:").unwrap(), 16088156);
    assert_eq!(meminfo_field(meminfo, "MemAvailable:").unwrap(), 1000000);
    assert!(meminfo_field(meminfo, "SwapTotal:").is_err());
  }

  #[test]
  fn runtime_mounts_are_excluded() {
    for mount in [
      "/run",
      "/run/user/1000",
      "/dev",
      "/dev/shm",
      "/nix",
      "/nix/store",
    ] {
      assert!(is_runtime_mount(mount), "{mount} must be excluded");
    }
    for mount in [
      "/",
      "/scratch",
      "/workspace",
      "/tmp",
      "/srv/run-like",
      "/devtools",
    ] {
      assert!(!is_runtime_mount(mount), "{mount} must not be excluded");
    }
  }

  #[test]
  fn mount_point_escapes_decode() {
    assert_eq!(unescape_mount_point("/"), "/");
    assert_eq!(
      unescape_mount_point("/srv/with\\040space"),
      "/srv/with space"
    );
    assert_eq!(unescape_mount_point("/srv/with\\011tab"), "/srv/with\ttab");
    assert_eq!(
      unescape_mount_point("/srv/with\\134slash"),
      "/srv/with\\slash"
    );
    // A dangling or non-octal escape is kept verbatim.
    assert_eq!(unescape_mount_point("/srv/with\\4"), "/srv/with\\4");
    assert_eq!(unescape_mount_point("/srv/with\\09z"), "/srv/with\\09z");
  }
}
