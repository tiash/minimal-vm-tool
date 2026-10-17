//! End-to-end test: the real agent binary against a local AF_VSOCK listener.
//!
//! Skips when the sandbox has no working vsock loopback (most dev sandboxes
//! and CI containers); the full-device coverage lives in molly-vm's QEMU
//! suite, which boots the actual guest.
mod support;

use std::time::Duration;

use minimal_vm_liveness_protocol as protocol;
use support::FakeHost;

#[tokio::test(flavor = "multi_thread")]
async fn reports_status_until_killed() {
  simple_test_logging::init();
  if !support::vsock_loopback_works() {
    eprintln!("skipping: AF_VSOCK loopback not usable in this sandbox");
    return;
  }

  let (listener, host) = FakeHost::bind().expect("binding fake host");
  let binary = env!("CARGO_BIN_EXE_minimal-vm-liveness-agent");
  let port = host.port.to_string();
  let mut child = tokio::process::Command::new(binary)
    .arg("--host-cid")
    .arg("1") // loopback dials the local CID, not the real host (2)
    .arg("--port")
    .arg(&port)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .expect("spawning the liveness agent");

  let (stream, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
    .await
    .expect("agent never connected")
    .expect("accepting the agent's connection");
  let mut rx = protocol::io::Rx::new(stream);

  let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
    .await
    .expect("no first report")
    .expect("reading first report")
    .expect("first report should be valid");
  assert!(first.cpu_percent >= 0.0 && first.cpu_percent <= 100.0);
  assert!(first.memory.total > 0);
  assert!(first.disk.total > 0);

  child.kill().await.expect("killing the agent");
  let _ = child.wait().await;
}
