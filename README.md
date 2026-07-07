# minimal-vm-tool

A very minimal VM agent that can be integrated into a Nix VM and driven from
Rust (or any other platform) over [virtio-vsock][vsock].

It lets a host spawn and control a single process inside a guest VM per
connection: streaming stdin/stdout/stderr, signalling, and reporting exit
status — all over a small line-delimited JSON protocol.

[vsock]: https://docs.oasis-open.org/virtio/virtio/v1.2/csprd01/virtio-v1.2-csprd01.html#x1-2900003

## Repository layout

This is a Cargo workspace with three members:

| Crate                           | Path          | Description                                                                       |
| ------------------------------- | ------------- | --------------------------------------------------------------------------------- |
| `minimal-vm-exec-protocol`      | [`protocol/`](protocol) | The wire protocol: message types, serde definitions, and async send/receive helpers (`io::Tx` / `io::Rx`). |
| `minimal-vm-exec-agent`         | [`agent/`](agent)       | The binary that runs **inside** the VM. Spawned per connection (e.g. by `inetd`/`systemd` socket activation) with the vsock connection on stdin/stdout. |
| `test-logger`                   | [`test-logger/`](test-logger) | Tiny test-only logger initializer used by the integration tests.                |

## The protocol

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

## Building and testing

Requires Rust 1.95.0 (see [`rust-toolchain.toml`](rust-toolchain.toml)).

```sh
# build everything
cargo build

# run the full test suite (protocol round-trips + agent end-to-end tests)
cargo test

# build the release agent binary
cargo build --release --bin minimal-vm-exec-agent
```

The agent's end-to-end tests (`agent/tests/end_to_end.rs`) boot the actual
`minimal-vm-exec-agent` binary and exchange messages with it.

## Nix packaging

[`agent/package.nix`](agent/package.nix) builds the agent as a Nix derivation
for inclusion in a VM image. It is designed to be called from the consuming
project's flake, e.g.:

```nix
minimal-vm-exec-agent = pkgs.callPackage ./minimal-vm-tool/agent/package.nix { };
```

It then typically runs under `systemd` socket activation on the `exec` vsock
port, one instance per connection.

## License

[MIT](LICENSE).
