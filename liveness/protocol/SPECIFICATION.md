# Minimal VM Liveness Protocol Specification

## Overview

The Minimal VM Liveness Protocol is a one-way status protocol implemented
over virtio-vsock. The agent inside the VM connects out to the host and
streams guest resource status (systemd state, CPU, memory, disk) so the host
can monitor liveness and gate on boot completion.

## Protocol Port

- **Port Name:** `"live"`
- **Port Number:** `1818850917` (big-endian encoding of `"live"` as u32)

## Connection Lifecycle

1. The AGENT (inside the VM) initiates a connection to the HOST (CID 2) on
   the liveness port. Unlike the exec protocol, the connection direction is
   **VM → HOST**: the agent dials as soon as its service starts, which may
   be long before the guest has finished booting.
2. The AGENT sends one `status` message immediately after connecting, then
   one every [`INTERVAL`] seconds while the connection is up.
3. On any error — write failure, host disconnect, shutdown signal — the
   AGENT exits. Reconnection is the service manager's job (the unit runs
   with `Restart=always`), not the agent's: each process handles exactly
   one connection.

The HOST should keep an accept loop on the port for the VM's lifetime and
treat every accepted connection as authoritative: the newest status report
is the VM's current status. Reports with a non-operational `systemd` state
(e.g. `starting`) arrive during boot; the first operational state
(`running` or `degraded`) is the "booted" signal.

[`INTERVAL`]: (2 seconds; see the protocol crate's `INTERVAL` constant)

## Message Format

All messages are JSON objects serialized as UTF-8 strings, terminated by a
newline (`\n`).

The only message is `status` (AGENT → HOST):

```json
{
  "status":
  {
    "systemd": "running",
    "cpu_percent": 12.5,
    "memory": { "used": 123456789, "total": 8589934592 },
    "disk": { "used": 23456789, "total": 987654321 }
  }
}
```

**Fields:**

- `systemd`: the guest systemd manager's overall state, one of
  `initializing`, `starting`, `running`, `degraded`, `maintenance`,
  `stopping`, or `unknown` (systemd offline, or a state the agent did not
  recognize). `running`/`degraded` mean the boot has completed.
- `cpu_percent`: guest CPU utilisation, 0–100 across all cores, measured
  over the interval since the previous report.
- `memory`: guest RAM usage in bytes. `used` is `MemTotal − MemAvailable`
  (what a new allocation would have to reclaim), `total` is `MemTotal`.
- `disk`: root filesystem usage in bytes, from `statvfs("/")`.

The HOST does not reply to status messages; the connection carries no
HOST → AGENT traffic.

## Behaviour notes

- The first report is sent immediately on connect (no interval wait), so a
  booting host sees `starting` as early as possible.
- `systemctl is-system-running` failing (systemd not started yet) maps to
  `systemd: "unknown"`, not to a dropped report: early-boot status must
  still reach the host.
- Metrics are collected from `/proc/stat`, `/proc/meminfo`, `statvfs` and
  `systemctl` only — no shell-outs beyond `systemctl`, no dependencies on
  services that start late.
