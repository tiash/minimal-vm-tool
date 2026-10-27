use crate::FileSystemUsage;
use crate::Status;
use crate::SystemdState;
use crate::Usage;
use crate::io::Rx;
use crate::io::Tx;

fn sample_status() -> Status {
  Status {
    systemd: SystemdState::Starting,
    cpu_percent: 42.0,
    memory: Usage { used: 1, total: 4 },
    swap: Usage { used: 2, total: 8 },
    disks: vec![FileSystemUsage {
      mount_point: "/".to_string(),
      label: Some("scratch".to_string()),
      usage: Usage { used: 2, total: 8 },
    }],
  }
}

#[tokio::test]
async fn tx_rx_round_trip() {
  let (client, server) = tokio::io::duplex(1024);
  let mut tx = Tx::new(client);
  let mut rx = Rx::new(server);

  tx.send(&sample_status()).await.expect("send");
  let received = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
    .await
    .expect("timed out")
    .expect("read")
    .expect("stream should still be open");
  assert_eq!(received, sample_status());
}

#[tokio::test]
async fn rx_sees_connection_close_as_none() {
  let (client, server) = tokio::io::duplex(1024);
  let mut tx = Tx::new(client);
  let mut rx = Rx::new(server);

  tx.send(&sample_status()).await.expect("send");
  let received = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
    .await
    .expect("timed out")
    .expect("read")
    .expect("stream should still be open");
  assert_eq!(received, sample_status());
  // Drained: drop the writer so the reader sees EOF.
  drop(tx);
  let received = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
    .await
    .expect("timed out")
    .expect("read");
  assert_eq!(received, None);
}

#[tokio::test]
async fn rx_rejects_invalid_lines() {
  let (client, server) = tokio::io::duplex(1024);
  use tokio::io::AsyncWriteExt as _;
  let mut client = client;
  client
    .write_all(b"{\"nonsense\": 1}\n")
    .await
    .expect("write");
  drop(client);
  let mut rx = Rx::new(server);
  let result = rx.recv().await;
  assert!(result.is_err(), "a non-status message must be an error");
}
