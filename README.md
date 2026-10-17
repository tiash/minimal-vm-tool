# minimal-vm-tool

A very minimal VM agent that can be integrated into a Nix VM and driven from
Rust (or any other platform) over [virtio-vsock][vsock].

It lets a host spawn and control a single process inside a guest VM per
connection: streaming stdin/stdout/stderr, signalling, and reporting exit
status — all over a small line-delimited JSON protocol. A companion liveness
agent streams guest resource status (systemd state, CPU, memory, disk) to
the host over a second vsock port.

[vsock]: https://docs.oasis-open.org/virtio/virtio/v1.2/csprd01/virtio-v1.2-csprd01.html#x1-2900003

## Repository layout

This is a Cargo workspace with these members:

| Crate                           | Path          | Description                                                                       |
| ------------------------------- | ------------- | --------------------------------------------------------------------------------- |
| `minimal-vm-exec-protocol`      | [`protocol/`](protocol) | The exec wire protocol: message types, serde definitions, and async send/receive helpers (`io::Tx` / `io::Rx`). |
| `minimal-vm-exec-agent`         | [`agent/`](agent)       | The binary that runs **inside** the VM. Spawned per connection (e.g. by `inetd`/`systemd` socket activation) with the vsock connection on stdin/stdout. |
| `minimal-vm-liveness-protocol`  | [`liveness/protocol/`](liveness/protocol) | The liveness wire protocol: the guest `Status` message and its async send/receive helpers. |
| `minimal-vm-liveness`           | [`liveness/`](liveness) | The liveness agent that runs **inside** the VM: connects out to the host once and streams status until disconnected (systemd restarts it to reconnect). |

The agents' integration tests use the external
[`simple-test-logging`](https://github.com/tiash/simple-test-logging) crate
(pulled in as a git dev-dependency) for log initialization.

## The exec protocol

Communication is newline-delimited JSON objects sent over a virtio-vsock
connection on a fixed port:

```text
PORT = u32::from_be_bytes(*b"exec") // = 1702389091
```

A typical exchange:

```text
HOST → VM: { "exec": { "prog": "echo", "args": [ "hello" ] } }
VM → HOST: { "started": { "pid": 1234 } }
VM → HOST: { "stdout": { "data": "hello\n" } }
VM → HOST: { "stdout": { "closed": true } }
VM → HOST: { "exited": { "status": 0 } }
HOST → VM: [close connection]
```

See [`protocol/SPECIFICATION.md`](protocol/SPECIFICATION.md) for the complete,
authoritative specification of every message (`exec`, `started`, `stdin`,
`stdout`, `stderr`, `kill`, `exited`, `error`) and the connection lifecycle.

The protocol crate is usable on its own from any Rust host that wants to talk
to a running agent.

## The liveness protocol

The liveness agent is the VM's heartbeat: it dials the host (CID 2) on a
fixed port

```text
PORT = u32::from_be_bytes(*b"live") // = 1818850917
```

and streams one status line every few seconds:

```text
VM → HOST: {"status":{"systemd":"running","cpu_percent":12.5,"memory":{"used":…,"total":…},"disk":{"used":…,"total":…}}}
```

The connection is made once. On disconnect or error the agent exits and
systemd's `Restart=always` reconnects it — reconnection policy lives in one
place (the service unit), not in the agent. `systemd: "running"` (or
`"degraded"`) is the host's signal that the VM has finished booting; a
consumed-by-booting host waits for the first operational state instead of a
one-shot ready message.

See [`liveness/protocol/SPECIFICATION.md`](liveness/protocol/SPECIFICATION.md)
for the full message specification.

## Building and testing

Requires Rust 1.98.1 (see [`rust-toolchain.toml`](rust-toolchain.toml)).

```sh
# build everything
cargo build

# run the full test suite (protocol round-trips + agent end-to-end tests)
cargo test

# build the release agent binaries
cargo build --release --bin minimal-vm-exec-agent
cargo build --release --bin minimal-vm-liveness-agent
```

The agents' end-to-end tests boot the actual agent binaries and exchange
messages with them (the liveness e2e needs an AF_VSOCK device and skips
when the sandbox has none).

## Nix packaging

[`agent/package.nix`](agent/package.nix) and
[`liveness/package.nix`](liveness/package.nix) build the agents as Nix
derivations for inclusion in a VM image. They are designed to be called from
the consuming project's flake, e.g.:

```nix
minimal-vm-exec-agent = pkgs.callPackage ./minimal-vm-tool/agent/package.nix { };
minimal-vm-liveness-agent = pkgs.callPackage ./minimal-vm-tool/liveness/package.nix { };
```

The exec agent typically runs under `systemd` socket activation on the `exec`
vsock port, one instance per connection; the liveness agent runs as a plain
`Restart=always` service that dials the host itself.

## License

[MIT](LICENSE).
