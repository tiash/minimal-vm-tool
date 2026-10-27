//! Agent runtime for the liveness protocol: the connect-report-exit loop,
//! independent of how the connection is obtained.
//!
//! Kept as a library module so the end-to-end test can drive the exact
//! loop the binary runs over a socketpair instead of a real vsock device.

use anyhow::Context;
use anyhow::Result;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::pin;

use minimal_vm_liveness_protocol as protocol;

/// One connection's worth of reporting: collect metrics every
/// [`protocol::INTERVAL`] and send them until the write fails (host gone).
///
/// Returns when the stream is unusable; the caller (the binary) then exits
/// and systemd reconnects by restarting the process. A dropped connection
/// is a normal event here — `Ok(())` — because there is nothing left to do
/// on the connection; only a local error (e.g. metrics collection) is `Err`.
pub async fn report_until_disconnected(
  stream: impl AsyncRead + AsyncWrite + Unpin,
  shutdown: impl std::future::Future<Output = ()>,
) -> Result<()> {
  let mut tx = protocol::io::Tx::new(stream);
  let mut cpu = crate::metrics::cpu_sample()?;
  pin!(shutdown);
  loop {
    // The first report goes out immediately: the host's boot gate polls
    // for an operational systemd state, and one interval is a long time to
    // add to every boot.
    let result = report(&mut tx, &mut cpu).await;
    if let Err(e) = result {
      log::info!("stopping reports: {e:#}");
      return Ok(());
    }
    tokio::select! {
      biased;
      // Shutdown first: exit promptly so the VM's stop path is not held up.
      _ = shutdown.as_mut() => {
        log::info!("shutdown signal: disconnecting");
        return Ok(());
      },
      _ = tokio::time::sleep(protocol::INTERVAL) => {},
    }
  }
}

/// Collect one report and send it.
async fn report(
  tx: &mut protocol::io::Tx<impl AsyncWrite + Unpin>,
  cpu: &mut crate::metrics::CpuSample,
) -> Result<()> {
  let status = protocol::Status {
    systemd: crate::metrics::systemd_state(),
    cpu_percent: crate::metrics::sample(cpu)?,
    memory: crate::metrics::memory()?,
    swap: crate::metrics::swap()?,
    disks: crate::metrics::disks()?,
  };
  tx.send(&status)
    .await
    .context("host connection lost while reporting")?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Drive the loop against an in-memory duplex pair and collect what it
  /// sends, without needing a vsock device (none exists in most dev sandboxes).
  #[tokio::test]
  async fn reports_over_a_stream_until_disconnect() {
    let (client, mut server) = tokio::io::duplex(1024);
    let (never_tx, never_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown = async move {
      let _ = never_rx.await;
    };
    let task = tokio::spawn(report_until_disconnected(client, shutdown));

    let first = {
      let mut rx = protocol::io::Rx::new(&mut server);
      tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for the first report")
        .expect("reading the first report")
        .expect("first report should be valid")
    };
    assert!(first.cpu_percent >= 0.0 && first.cpu_percent <= 100.0);
    assert!(first.memory.total > 0);
    assert!(first.swap.used <= first.swap.total);
    assert!(
      first.disks.iter().any(|d| d.mount_point == "/"),
      "the root filesystem must be reported"
    );

    // Dropping the server end of the duplex closes the stream: the next
    // send must fail and the loop must return.
    drop(server);
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), task)
      .await
      .expect("loop did not exit after disconnect")
      .expect("report task panicked");
    assert!(result.is_ok(), "disconnect is not an error: {result:?}");
    never_tx.send(()).ok();
  }
}
