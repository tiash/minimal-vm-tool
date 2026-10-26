use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Context;
use std::task::Poll;

use tokio::io::AsyncWrite;

use crate::Exited;
use crate::Message;
use crate::io::Tx;

/// A writer whose acceptance is gated by a flag: while stuck, every write
/// parks (registering its waker); once released, everything flows. Mirrors a
/// connection under backpressure — e.g. a full vsock credit window — without
/// needing a real transport.
#[derive(Clone)]
struct Gated {
  state: Arc<Mutex<GatedState>>,
}

impl Default for Gated {
  fn default() -> Self {
    Self {
      state: Arc::new(Mutex::new(GatedState {
        stuck: true,
        ..GatedState::default()
      })),
    }
  }
}

#[derive(Default)]
struct GatedState {
  stuck: bool,
  written: Vec<u8>,
  waker: Option<std::task::Waker>,
}

impl Gated {
  fn release(&self) {
    let mut state = self.state.lock().expect("gated state");
    state.stuck = false;
    if let Some(waker) = state.waker.take() {
      waker.wake();
    }
  }

  fn written(&self) -> Vec<u8> {
    self.state.lock().expect("gated state").written.clone()
  }
}

impl AsyncWrite for Gated {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8],
  ) -> Poll<std::io::Result<usize>> {
    let mut state = self.state.lock().expect("gated state");
    if state.stuck {
      state.waker = Some(cx.waker().clone());
      return Poll::Pending;
    }
    state.written.extend_from_slice(buf);
    Poll::Ready(Ok(buf.len()))
  }

  fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
    Poll::Ready(Ok(()))
  }

  fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
    Poll::Ready(Ok(()))
  }
}

fn exited() -> Message {
  Message::Exited(Exited { status: 0 })
}

/// `send` reports success even while the connection accepts nothing: the
/// message stays in the Tx buffer until some later Tx call retries the write.
/// A quiet connection (a terminal message nothing follows) would never
/// deliver it — which is why terminal messages must use `send_and_flush`.
#[tokio::test]
async fn send_succeeds_with_the_message_undelivered_under_backpressure() {
  let gate = Gated::default();
  let mut tx = Tx::new(gate.clone());

  tokio::time::timeout(std::time::Duration::from_secs(1), tx.send(&[exited()]))
    .await
    .expect("send must not wait for the connection")
    .expect("send must succeed");

  assert!(
    gate.written().is_empty(),
    "send must not have delivered anything"
  );
}

/// `send_and_flush` stays pending until the message is actually on the
/// connection, then completes.
#[tokio::test]
async fn send_and_flush_waits_for_delivery() {
  let gate = Gated::default();
  let mut tx = Tx::new(gate.clone());

  let send = tokio::spawn(async move { tx.send_and_flush(&[exited()]).await });
  tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  assert!(
    !send.is_finished(),
    "send_and_flush must stay pending while the connection accepts nothing"
  );

  gate.release();
  tokio::time::timeout(std::time::Duration::from_secs(1), send)
    .await
    .expect("send_and_flush must complete once the connection accepts writes")
    .expect("join")
    .expect("send_and_flush must succeed");
  assert!(
    String::from_utf8_lossy(&gate.written()).contains(r#""exited""#),
    "the message must have been written: {:?}",
    String::from_utf8_lossy(&gate.written())
  );
}
