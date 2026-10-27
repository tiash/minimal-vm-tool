//! Async newline-delimited JSON transport for the liveness protocol.

use anyhow::Context;
use anyhow::Result;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::io::BufWriter;

use crate::Message;
use crate::Status;

/// Send side of a liveness connection.
pub struct Tx<W>
where
  W: tokio::io::AsyncWrite + Unpin,
{
  writer: BufWriter<W>,
}

impl<W: tokio::io::AsyncWrite + Unpin> Tx<W> {
  pub fn new(writer: W) -> Self {
    Self {
      writer: BufWriter::new(writer),
    }
  }

  /// Serialize and write one status line.
  pub async fn send(&mut self, status: &Status) -> Result<()> {
    let line =
      serde_json::to_string(&Message::Status(status.clone())).context("serializing status")?;
    self
      .writer
      .write_all(line.as_bytes())
      .await
      .context("writing status")?;
    self
      .writer
      .write_all(b"\n")
      .await
      .context("writing status newline")?;
    self.writer.flush().await.context("flushing status")?;
    Ok(())
  }
}

/// Receive side of a liveness connection.
pub struct Rx<R>
where
  R: tokio::io::AsyncRead + Unpin,
{
  lines: tokio::io::Lines<BufReader<R>>,
}

impl<R: tokio::io::AsyncRead + Unpin> Rx<R> {
  pub fn new(reader: R) -> Self {
    Self {
      lines: BufReader::new(reader).lines(),
    }
  }

  /// Read the next status. Returns `Ok(None)` on connection close.
  pub async fn recv(&mut self) -> Result<Option<Status>> {
    match self.lines.next_line().await {
      | Ok(Some(line)) => {
        let msg: Message = serde_json::from_str(&line).context("deserializing liveness message")?;
        match msg {
          | Message::Status(status) => Ok(Some(status)),
        }
      },
      | Ok(None) => Ok(None),
      | Err(e) => Err(anyhow::anyhow!(e).context("reading liveness line")),
    }
  }
}

#[cfg(test)]
mod tests;
