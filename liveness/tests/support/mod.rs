//! Test-only fake host: an AF_VSOCK listener the agent can dial.
use std::sync::OnceLock;

/// Whether AF_VSOCK loopback works in this sandbox: a listener bound on the
/// wildcard CID must be reachable by a local dial. Guest vsock stacks
/// without the `vsock_loopback` transport reset self-connects, and
/// unprivileged sockets may not bind at all — either way the e2e cannot
/// run here (the full-device coverage lives in molly-vm's QEMU suite).
pub fn vsock_loopback_works() -> bool {
  static LOOPBACK: OnceLock<bool> = OnceLock::new();
  *LOOPBACK.get_or_init(|| probe().is_ok())
}

/// Bind an ephemeral port on the wildcard CID and dial it locally.
fn probe() -> anyhow::Result<()> {
  let listener = socket2::Socket::new(socket2::Domain::VSOCK, socket2::Type::STREAM, None)?;
  listener.set_nonblocking(true)?;
  listener.bind(&socket2::SockAddr::vsock(libc::VMADDR_CID_ANY, 0))?;
  listener.listen(1)?;
  let port = listener
    .local_addr()?
    .as_vsock_address()
    .expect("vsock local address")
    .1;
  let dialer = socket2::Socket::new(socket2::Domain::VSOCK, socket2::Type::STREAM, None)?;
  dialer.set_nonblocking(true)?;
  dialer.connect(&socket2::SockAddr::vsock(libc::VMADDR_CID_HOST, port))?;
  let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
  loop {
    match listener.accept() {
      | Ok(_) => return Ok(()),
      | Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
        anyhow::ensure!(
          std::time::Instant::now() < deadline,
          "loopback dial never completed"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
      },
      | Err(e) => return Err(e.into()),
    }
  }
}

pub struct FakeHost {
  /// Duplicate of the listener fd: keeps the bound port alive even though
  /// the socket itself was moved into the tokio listener.
  _keepalive: std::os::fd::OwnedFd,
  /// The ephemeral port the agent should dial.
  pub port: u32,
}

impl FakeHost {
  /// Bind an AF_VSOCK listener on an ephemeral port of the wildcard CID.
  pub fn bind() -> anyhow::Result<(tokio::net::UnixListener, Self)> {
    let socket = socket2::Socket::new(socket2::Domain::VSOCK, socket2::Type::STREAM, None)?;
    socket.set_nonblocking(true)?;
    // The wildcard CID with port 0 asks the kernel for a free port.
    socket.bind(&socket2::SockAddr::vsock(libc::VMADDR_CID_ANY, 0))?;
    socket.listen(1)?;
    let addr = socket
      .local_addr()?
      .as_vsock_address()
      .expect("vsock local address");
    let port = addr.1;
    let keepalive = socket.try_clone()?;
    let listener = tokio::net::UnixListener::from_std(socket.into())?;
    Ok((
      listener,
      Self {
        _keepalive: keepalive.into(),
        port,
      },
    ))
  }
}
