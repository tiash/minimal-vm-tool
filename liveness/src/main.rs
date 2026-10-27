use anyhow::Context;
use anyhow::Result;
use clap::Parser;
use minimal_vm_liveness_protocol as protocol;

/// Command-line arguments for the minimal-vm-liveness-agent.
#[derive(clap::Parser, Debug)]
#[command(name = "minimal-vm-liveness-agent")]
#[command(
  about = "Minimal VM Liveness Agent",
  long_about = "Connects to the host over virtio-vsock and streams one status line (systemd state, CPU, memory, swap, per-filesystem usage) every interval. The connection is made once: on disconnect or error the process exits, and systemd's Restart= policy reconnects it."
)]
struct Args {
  /// CID to connect to. The default is the vsock host CID: the agent always
  /// runs inside the guest, where the host is CID 2.
  #[arg(long, default_value_t = libc::VMADDR_CID_HOST)]
  host_cid: u32,

  /// vsock port to connect to. The default is the fixed liveness port; the
  /// e2e tests override it to reach a local listener instead.
  #[arg(long, default_value_t = protocol::PORT)]
  port: u32,

  /// Increase logging verbosity above the default (`Info`).
  /// `-v` selects `Debug`, `-vv` selects `Trace`.
  #[arg(short, long, action = clap::ArgAction::Count)]
  verbose: u8,

  /// Decrease logging verbosity below the default (`Info`).
  /// `-q` selects `Warn`, `-qq` selects `Error`.
  #[arg(short, long, action = clap::ArgAction::Count)]
  quiet: u8,
}

/// Resolve the `-v` / `-q` flag counts into a `stderrlog` verbosity level.
fn verbosity_from_flags(verbose: u8, quiet: u8) -> stderrlog::LogLevelNum {
  match verbose as i16 - quiet as i16 {
    | n if n <= -2 => stderrlog::LogLevelNum::Error,
    | -1 => stderrlog::LogLevelNum::Warn,
    | 0 => stderrlog::LogLevelNum::Info,
    | 1 => stderrlog::LogLevelNum::Debug,
    | _ => stderrlog::LogLevelNum::Trace,
  }
}

#[tokio::main]
async fn main() {
  let args = Args::parse();

  stderrlog::new()
    .verbosity(verbosity_from_flags(args.verbose, args.quiet))
    .timestamp(stderrlog::Timestamp::Millisecond)
    .color(stderrlog::ColorChoice::Auto)
    .show_module_names(true)
    .init()
    .expect("Failed to initialize logger");

  log::info!("Minimal VM Liveness Agent starting with args: {:?}", args);

  match run(&args).await {
    | Ok(()) => log::info!("Liveness agent exiting cleanly"),
    | Err(e) => {
      log::error!("{e:#}");
      std::process::exit(1);
    },
  }
}

/// Connect once, report until disconnected. Every exit path — error, EOF,
/// or shutdown signal — ends the process so systemd restarts it with a
/// fresh connection.
async fn run(args: &Args) -> Result<()> {
  let mut shutdown = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    .expect("installing SIGTERM handler");

  let stream = connect(args.host_cid, args.port).await?;
  log::info!("Connected to host CID {} port {}", args.host_cid, args.port);

  let shutdown = async move {
    shutdown.recv().await;
  };
  agent::report_until_disconnected(stream, shutdown).await
}

/// Dial the host. The connect is non-blocking: `connect()` usually returns
/// `EINPROGRESS` (the vsock transport completes asynchronously), so the
/// socket is converted to tokio and the completion is awaited via
/// writability; `take_error()` then surfaces an async failure (e.g. a host
/// RST) instead of reporting on a dead connection.
async fn connect(cid: u32, port: u32) -> Result<tokio::net::UnixStream> {
  let socket = socket2::Socket::new(socket2::Domain::VSOCK, socket2::Type::STREAM, None)?;
  socket.set_nonblocking(true)?;
  let addr = socket2::SockAddr::vsock(cid, port);
  match socket.connect(&addr) {
    | Ok(()) => {},
    // The connect is in flight; its outcome is polled below.
    | Err(e) if e.raw_os_error() == Some(libc::EINPROGRESS) => {},
    | Err(e) => return Err(anyhow::anyhow!("initiating vsock connect: {e}")),
  }
  let stream = tokio::net::UnixStream::from_std(socket.into())?;
  stream
    .writable()
    .await
    .context("waiting for vsock connect to complete")?;
  match stream.take_error()? {
    | None => Ok(stream),
    | Some(e) => Err(anyhow::anyhow!("vsock connect failed: {e}")),
  }
}

mod agent;
mod metrics;
