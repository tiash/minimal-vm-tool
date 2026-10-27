//! Guest liveness and resource-status protocol.
//!
//! The agent running inside the VM connects out to the host (CID 2) on this
//! port and streams one [`Status`] per [`INTERVAL`]. The host keeps the
//! latest value available for the VM's lifetime; the first [`SystemdState`]
//! that reports an operational state is the "booted" signal, replacing the
//! old one-shot `READY` message.
//!
//! Messages are newline-delimited JSON, exactly like the exec protocol.

pub mod io;

use serde::Deserialize;
use serde::Serialize;

/// vsock port the liveness agent dials on the host. Big-endian encoding of
/// `b"live"` — the same scheme as the exec protocol's port (`b"exec"`).
pub const PORT: u32 = u32::from_be_bytes(*b"live");

/// Seconds between the status reports the agent streams while connected.
pub const INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

#[test]
fn show_port() {
  expect_test::expect![[r#"1818850917"#]].assert_eq(&format!("{}", PORT));
}

/// systemd's overall system state (`systemctl is-system-running`).
///
/// `is-system-running` exits non-zero for every state but `running` and
/// prints `offline` (or nothing) when systemd is not running at all, so the
/// agent maps unknown outputs to [`SystemdState::Unknown`] rather than
/// failing the report — a booting or degraded system must still produce
/// status lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SystemdState {
  /// `initializing` — boot started, units not yet queued.
  Initializing,
  /// `starting` — boot in progress (the state the old `READY` gate raced).
  Starting,
  /// `running` — boot complete, everything operational.
  Running,
  /// `degraded` — running, but some units failed.
  Degraded,
  /// `maintenance` — a maintenance unit is active.
  Maintenance,
  /// `stopping` — shutdown in progress.
  Stopping,
  /// Anything else: `offline`, no output, or a state that is not listed.
  Unknown,
}

impl SystemdState {
  /// Whether systemd considers the system operational (`running` or
  /// `degraded`): the host's "booted" condition. `degraded` counts — some
  /// units may have failed, but the manager finished booting and services
  /// (in particular the exec agent) are up.
  pub fn is_operational(self) -> bool {
    matches!(self, SystemdState::Running | SystemdState::Degraded)
  }

  /// Parse the raw output of `systemctl is-system-running` (single line).
  pub fn parse(output: &str) -> Self {
    match output.trim() {
      | "initializing" => SystemdState::Initializing,
      | "starting" => SystemdState::Starting,
      | "running" => SystemdState::Running,
      | "degraded" => SystemdState::Degraded,
      | "maintenance" => SystemdState::Maintenance,
      | "stopping" => SystemdState::Stopping,
      | _ => SystemdState::Unknown,
    }
  }
}

impl std::fmt::Display for SystemdState {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let word = match self {
      | SystemdState::Initializing => "initializing",
      | SystemdState::Starting => "starting",
      | SystemdState::Running => "running",
      | SystemdState::Degraded => "degraded",
      | SystemdState::Maintenance => "maintenance",
      | SystemdState::Stopping => "stopping",
      | SystemdState::Unknown => "unknown",
    };
    f.write_str(word)
  }
}

/// One status report from the guest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
  /// systemd's overall state: the boot gate (`is_operational`) and the
  /// health signal the host monitors.
  pub systemd: SystemdState,
  /// CPU load, 0–100% across all cores.
  pub cpu_percent: f64,
  /// RAM: used / total bytes.
  pub memory: Usage,
  /// Swap: used / total bytes. Defaults to zeroes: a guest built from an
  /// older revision omits the field, and the host must still accept its
  /// reports (images are cached and upgraded independently of the host).
  #[serde(default)]
  pub swap: Usage,
  /// Every writable guest filesystem: one entry per mount point. Defaults
  /// to empty for the same reason as [`Status::swap`].
  #[serde(default)]
  pub disks: Vec<FileSystemUsage>,
}

/// One guest filesystem's usage, keyed by its mount point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSystemUsage {
  /// The mount point, as it appears in `/proc/mounts`.
  pub mount_point: String,
  /// Used / total bytes on that filesystem.
  pub usage: Usage,
}

/// Used/total byte counts of one resource.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
  pub used: u64,
  pub total: u64,
}

/// The only message the agent sends: `{"status": ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Message {
  Status(Status),
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn systemd_state_parses_is_system_running_output() {
    expect_test::expect![[r#"Starting"#]]
      .assert_eq(&format!("{:?}", SystemdState::parse("starting\n")));
    expect_test::expect![[r#"Running"#]]
      .assert_eq(&format!("{:?}", SystemdState::parse("running")));
    expect_test::expect![[r#"Degraded"#]]
      .assert_eq(&format!("{:?}", SystemdState::parse("degraded")));
    // `is-system-running` prints nothing at all when systemd is offline.
    expect_test::expect![[r#"Unknown"#]].assert_eq(&format!("{:?}", SystemdState::parse("")));
    expect_test::expect![[r#"Unknown"#]]
      .assert_eq(&format!("{:?}", SystemdState::parse("nonsense")));
  }

  #[test]
  fn systemd_state_displays_as_the_raw_word() {
    expect_test::expect![[r#"degraded"#]].assert_eq(&SystemdState::Degraded.to_string());
    expect_test::expect![[r#"unknown"#]].assert_eq(&SystemdState::Unknown.to_string());
  }

  #[test]
  fn operational_states_are_running_and_degraded() {
    assert!(SystemdState::Running.is_operational());
    assert!(SystemdState::Degraded.is_operational());
    assert!(!SystemdState::Starting.is_operational());
    assert!(!SystemdState::Unknown.is_operational());
  }

  #[test]
  fn status_round_trips_through_json() {
    let status = Status {
      systemd: SystemdState::Starting,
      cpu_percent: 12.5,
      memory: Usage { used: 1, total: 2 },
      swap: Usage { used: 0, total: 0 },
      disks: vec![FileSystemUsage {
        mount_point: "/".to_string(),
        usage: Usage { used: 3, total: 4 },
      }],
    };
    let line = serde_json::to_string(&Message::Status(status.clone())).expect("serialize");
    expect_test::expect![[r#"
    {"status":{"systemd":"starting","cpu_percent":12.5,"memory":{"used":1,"total":2},"swap":{"used":0,"total":0},"disks":[{"mount_point":"/","usage":{"used":3,"total":4}}]}}"#]]
    .assert_eq(&line);
    let parsed = serde_json::from_str::<Message>(&line).expect("deserialize");
    assert_eq!(parsed, Message::Status(status));
  }

  #[test]
  fn old_guest_report_without_swap_and_disks_still_parses() {
    let line = r#"{"status":{"systemd":"running","cpu_percent":5.0,"memory":{"used":1,"total":2},"disk":{"used":3,"total":4}}}"#;
    let parsed = serde_json::from_str::<Message>(line).expect("deserialize");
    let Message::Status(status) = parsed;
    assert_eq!(status.systemd, SystemdState::Running);
    assert_eq!(status.swap, Usage::default());
    assert!(status.disks.is_empty());
  }

  #[test]
  fn unknown_systemd_state_round_trips() {
    let status = Status {
      systemd: SystemdState::Unknown,
      cpu_percent: 0.0,
      memory: Usage { used: 0, total: 0 },
      swap: Usage { used: 0, total: 0 },
      disks: Vec::new(),
    };
    let line = serde_json::to_string(&Message::Status(status.clone())).expect("serialize");
    let parsed = serde_json::from_str::<Message>(&line).expect("deserialize");
    assert_eq!(parsed, Message::Status(status));
  }
}
