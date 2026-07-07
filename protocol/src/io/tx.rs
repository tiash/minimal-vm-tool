use super::ArcErr;
use crate::Message;
use anyhow::Context as _;
use anyhow::Result;
use anyhow::anyhow;
use itertools::Itertools;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Poll;
use std::task::ready;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt as _;
pub struct Sending(Option<Vec<u8>>);
impl Sending {
  pub fn new<'a>(msg: impl Into<&'a Message>) -> Result<Self> {
    Ok(Self(Some(
      serde_json::to_vec(msg.into()).context("Failed to serialize message")?,
    )))
  }
}

pub struct TxLocal<W>
where
  W: AsyncWrite + Unpin,
{
  writer: W,
  transmitting: Vec<u8>,
}

impl<W: AsyncWrite + Unpin> TxLocal<W> {
  fn new(writer: W) -> Self {
    Self {
      writer,
      transmitting: Vec::new(),
    }
  }
  fn poll_transmit(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<()>> {
    let this = self.get_mut();
    while !this.transmitting.is_empty() {
      let n = ready!(Pin::new(&mut this.writer).poll_write(cx, &this.transmitting,))
        .context("Failed to write to connection")?;

      if n == 0 {
        return Poll::Ready(Err(anyhow!("Connection closed while sending message")));
      } else {
        this.transmitting.drain(..n);
      }
    }
    Poll::Ready(Ok(()))
  }
  fn poll_flush(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<()>> {
    let this = self.get_mut();
    let transmitted = Pin::new(&mut *this).poll_transmit(cx);
    let flushed = Pin::new(&mut this.writer).poll_flush(cx);

    match (transmitted, flushed) {
      | (Poll::Ready(Ok(())), Poll::Ready(Ok(()))) => Poll::Ready(Ok(())),
      | (Poll::Ready(Err(e)), _) => Poll::Ready(Err(e)),
      | (_, Poll::Ready(Err(e))) => Poll::Ready(Err(e).context("Failed to flush connection")),
      | (Poll::Pending, _) | (_, Poll::Pending) => Poll::Pending,
    }
  }
  fn poll_flush_in_bg(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<()>> {
    let this = self.get_mut();
    match Pin::new(&mut *this).poll_flush(cx) {
      | Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
      | Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
      | Poll::Pending => {
        #[cfg(not(test))]
        if this.transmitting.len() >= 1024 * 1024 {
          // Pushback
          Poll::Pending
        } else {
          Poll::Ready(Ok(()))
        }
        #[cfg(test)]
        Poll::Ready(Ok(()))
      },
    }
  }

  pub fn poll_send(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
    msg: &mut Sending,
  ) -> Poll<Result<()>> {
    let this = self.get_mut();
    ready!(Pin::new(&mut *this).poll_flush_in_bg(cx))?;
    match msg.0.take() {
      | None => {},
      | Some(m) => {
        this.transmitting.extend_from_slice(&m);
        this.transmitting.extend_from_slice(b"\n");
      },
    }
    ready!(Pin::new(&mut *this).poll_flush_in_bg(cx))?;
    Poll::Ready(Ok(()))
  }

  async fn close(mut self) -> Result<W> {
    while !self.transmitting.is_empty() {
      match self.writer.write(&self.transmitting).await {
        | Ok(0) => {
          log::error!(
            "Connection closed while closing Tx, remaining data will be lost: {:?}",
            String::from_utf8_lossy(&self.transmitting)
          );
          break;
        },
        | Ok(n) => {
          self.transmitting.drain(..n);
        },
        | Err(err) => {
          log::debug!(
            "Failed to flush remaining data while closing Tx: {err:?}, remaining data will be lost: {:?}",
            String::from_utf8_lossy(&self.transmitting)
          );
          break;
        },
      }
    }
    self
      .writer
      .shutdown()
      .await
      .context("Failed to shutdown connection")?;
    Ok(self.writer)
  }
}

pub struct Tx<W>(Arc<Mutex<TxLocalDrop<W>>>)
where
  W: AsyncWrite + Unpin + Send + Sync + 'static;
impl<W: Unpin> Unpin for Tx<W> where W: AsyncWrite + Unpin + Send + Sync + 'static {}
impl<W> Clone for Tx<W>
where
  W: AsyncWrite + Unpin + Send + Sync + 'static,
{
  fn clone(&self) -> Self {
    Self(self.0.clone())
  }
}

impl<W> Tx<W>
where
  W: AsyncWrite + Unpin + Send + Sync + 'static,
{
  pub fn new(writer: W) -> Self {
    Self(Arc::new(Mutex::new(TxLocalDrop(Ok(TxLocal::new(writer))))))
  }

  pub fn poll_flush(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Result<()>> {
    let Ok(mut guard) = self.0.lock() else {
      return Poll::Ready(Err(anyhow!("BUG: Tx Mutex broken")));
    };
    match &mut guard.0 {
      | Ok(tx) => Pin::new(tx).poll_flush(cx),
      | Err(e) => Poll::Ready(Err(e.into())),
    }
  }
  pub fn poll_send(
    self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
    msg: &mut Sending,
  ) -> Poll<Result<()>> {
    let Ok(mut guard) = self.0.lock() else {
      return Poll::Ready(Err(anyhow!("BUG: Tx Mutex broken")));
    };
    match &mut guard.0 {
      | Ok(tx) => Pin::new(tx).poll_send(cx, msg),
      | Err(e) => Poll::Ready(Err(e.into())),
    }
  }
  pub fn send<'a>(
    &mut self,
    msg: impl IntoIterator<Item = &'a Message>,
  ) -> impl Future<Output = Result<()>> + Send + Sync {
    DoSend {
      tx: self.clone(),
      msgs: DoSend::<W>::prepare_msgs(msg),
      flush: false,
    }
  }
  pub fn send_and_flush<'a>(
    &mut self,
    msg: impl IntoIterator<Item = &'a Message>,
  ) -> impl Future<Output = Result<()>> + Send + Sync {
    DoSend {
      tx: self.clone(),
      msgs: DoSend::<W>::prepare_msgs(msg),
      flush: true,
    }
  }
  pub fn flush(&mut self) -> impl Future<Output = Result<()>> + Send + Sync {
    DoSend {
      tx: self.clone(),
      msgs: Ok(Vec::new()),
      flush: true,
    }
  }
  #[allow(clippy::await_holding_lock)]
  pub async fn close(mut self) -> Result<W> {
    let () = self.flush().await?;

    let Ok(mut guard) = self.0.lock() else {
      return Err(anyhow!("BUG: Tx Mutex broken"));
    };
    let this = std::mem::replace(
      &mut guard.0,
      Err(ArcErr::from_anyhow(anyhow!("Tx was closed"))),
    )?;
    drop(guard);
    this.close().await
  }
}

struct DoSend<W>
where
  W: AsyncWrite + Unpin + Send + Sync + 'static,
{
  tx: Tx<W>,
  msgs: std::result::Result<Vec<Sending>, ArcErr>,
  flush: bool,
}

impl<W> DoSend<W>
where
  W: AsyncWrite + Unpin + Send + Sync + 'static,
{
  fn prepare_msgs<'a>(
    msg: impl IntoIterator<Item = &'a Message>,
  ) -> std::result::Result<Vec<Sending>, ArcErr> {
    let mut error = None;
    let msgs = msg
      .into_iter()
      .filter_map(|m| {
        if error.is_some() {
          None
        } else {
          match Sending::new(m) {
            | Ok(s) => Some(s),
            | Err(e) => {
              error = Some(e.context(format!("Failed to prepare message for sending: {m:?}")));
              None
            },
          }
        }
      })
      .collect_vec();
    if let Some(e) = error {
      Err(ArcErr::from_anyhow(e))
    } else {
      Ok(msgs)
    }
  }
}
impl<W> Future for DoSend<W>
where
  W: AsyncWrite + Unpin + Send + Sync,
{
  type Output = Result<()>;

  fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
    let Self { tx, msgs, flush } = self.get_mut();
    let msgs = match msgs {
      | Err(e) => return Poll::Ready(Err(e.clone().into())),
      | Ok(msgs) => msgs,
    };
    while let Some(msg) = msgs.first_mut() {
      ready!(Pin::new(&mut *tx).poll_send(cx, msg))?;
      assert!(msg.0.is_none());
      msgs.remove(0);
    }
    if *flush {
      ready!(Pin::new(&mut *tx).poll_flush(cx))?;
    }
    Poll::Ready(Ok(()))
  }
}

struct TxLocalDrop<W>(std::result::Result<TxLocal<W>, ArcErr>)
where
  W: AsyncWrite + Send + Unpin + 'static;

impl<W> TxLocalDrop<W> where W: AsyncWrite + Send + Unpin + 'static {}

impl<W> Drop for TxLocalDrop<W>
where
  W: AsyncWrite + Send + Unpin + 'static,
{
  fn drop(&mut self) {
    let Ok(this) = std::mem::replace(
      &mut self.0,
      Err(ArcErr::from_anyhow(anyhow!("Tx was dropped"))),
    ) else {
      return;
    };
    log::debug!(
      "Warning: Tx was dropped without being closed. Attempting to close in background..."
    );
    tokio::spawn(async move {
      match this.close().await {
        | Ok(_) => log::debug!("Tx closed successfully in background"),
        | Err(e) => log::error!("Failed to close Tx in background: {e:?}"),
      }
    });
  }
}
