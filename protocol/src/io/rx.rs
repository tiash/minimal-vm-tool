use std::{
    mem,
    ops::DerefMut,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Poll, ready},
};

use tokio::io::{AsyncRead, ReadBuf};

use crate::FromMessage;
use crate::Message;
use anyhow::anyhow;
use anyhow::{Context, Result};

#[derive(Clone)]
struct MsgBuffer {
    queue: Vec<Message>,
}
impl MsgBuffer {
    fn new() -> Self {
        Self { queue: Vec::new() }
    }
    fn push(&mut self, msg: Message) {
        self.queue.push(msg);
    }
    fn pop<M: FromMessage>(&mut self) -> Result<Option<M>> {
        for ix in 0..self.queue.len() {
            let msg = mem::replace(&mut self.queue[ix], Message::NoOp);
            match M::from_message(msg) {
                Ok(m) => {
                    let _ = self.queue.remove(ix);
                    return Ok(Some(m));
                }
                Err(msg) => {
                    let _ = mem::replace(&mut self.queue[ix], msg);
                    continue;
                }
            }
        }
        Ok(None)
    }
    fn close(&mut self) -> Vec<Message> {
        mem::take(&mut self.queue)
    }
}

struct MsgReader<R>
where
    R: AsyncRead + Unpin,
{
    reader: R,
    buffer: Vec<u8>,
}
impl<R> Unpin for MsgReader<R> where R: AsyncRead + Unpin {}

impl<R> MsgReader<R>
where
    R: AsyncRead + Unpin,
{
    fn new(reader: R) -> Self {
        Self {
            reader,
            buffer: Vec::with_capacity(1024),
        }
    }
    fn poll_read_message(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<Option<Message>>> {
        log::trace!(
            "Rx: Polling for message, current buffer: {:?}",
            String::from_utf8_lossy(&self.buffer)
        );
        let Self { reader, buffer } = self.as_mut().get_mut();
        loop {
            match buffer.iter().position(|&c| c == b'\n') {
                Some(0) => {
                    log::error!("Rx: found newline at position 0, skipping it");
                    // empty line, just skip it
                    buffer.drain(..1);
                    continue;
                }
                Some(ix) => {
                    // found new line, so we can parse a message
                    let msg = serde_json::de::from_slice(&buffer[0..ix])
                        .context("Failed to parse message");
                    log::trace!("Rx: Parsed message: {:?}", msg);
                    buffer.drain(..=ix);
                    return Poll::Ready(msg.map(Some).context("Failed to parse message"));
                }
                None => {
                    log::trace!("Rx: did not find newline, need to read more data");
                    let orig_buffer_len = buffer.len();
                    // make sure we have some capacity to read into
                    buffer.reserve(1024);
                    let bytes_read = {
                        let mut read_buf = ReadBuf::uninit(buffer.spare_capacity_mut());
                        ready!(Pin::new(&mut *reader).poll_read(cx, &mut read_buf))
                            .context("Failed to read from reader")?;
                        read_buf.filled().len()
                    };
                    // actually make the read bytes visible in the buffer
                    // SAFETY: `buffer.reserve(1024)` ensured at least 1024 bytes of
                    // spare capacity. `ReadBuf::uninit(buffer.spare_capacity_mut())`
                    // views exactly that spare capacity (uninit memory). `poll_read`
                    // filled the first `bytes_read` of it and reported that count via
                    // `read_buf.filled().len()`. We only expose those `bytes_read`
                    // initialized bytes by setting len to `orig_buffer_len + bytes_read`,
                    // which stays within the reserved capacity. The bytes between
                    // `orig_buffer_len` and `orig_buffer_len + bytes_read` are the only
                    // ones `poll_read` initialized, so no uninitialized memory is read.
                    unsafe { buffer.set_len(orig_buffer_len + bytes_read) };
                    if bytes_read != 0 {
                        log::trace!("Rx: read {} bytes, looping around", bytes_read);
                        continue;
                    } else {
                        // got eof
                        if buffer.is_empty() {
                            log::trace!("Rx: reached EOF - buffer is empty, returning None");
                            // and empty buffer, so we're done
                            return Poll::Ready(Ok(None));
                        } else {
                            log::error!(
                                "Rx: reached EOF - buffer is not empty, pushing newline to trigger parsing of remaining data"
                            );
                            // push a newline to trigger parsing of any remaining data
                            buffer.extend_from_slice(b"\n");
                            continue;
                        }
                    }
                }
            }
        }
    }
    fn close(&mut self) -> Vec<u8> {
        mem::take(&mut self.buffer)
    }
}

struct RxLocal<R>
where
    R: AsyncRead + Unpin,
{
    reader: MsgReader<R>,
    msgs: MsgBuffer,
}
impl<R> Unpin for RxLocal<R> where R: AsyncRead + Unpin {}

impl<R> RxLocal<R>
where
    R: AsyncRead + Unpin,
{
    fn new(reader: R) -> Self {
        Self {
            reader: MsgReader::new(reader),
            msgs: MsgBuffer::new(),
        }
    }

    fn poll_recv<M: FromMessage>(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<Option<M>>> {
        log::trace!(
            "Rx: Polling for message of type {}",
            std::any::type_name::<M>()
        );
        // Special case for Message, to avoid unnecessary buffering
        if let Some(m) = self.msgs.pop::<M>()? {
            log::trace!(
                "Rx: Found message of type {} in buffer, returning it",
                std::any::type_name::<M>()
            );
            return Poll::Ready(Ok(Some(m)));
        }
        log::trace!(
            "Rx: did not find message of type {} in buffer",
            std::any::type_name::<M>()
        );
        loop {
            match ready!(Pin::new(&mut self.reader).poll_read_message(cx))? {
                None => {
                    log::trace!(
                        "Rx: Received None while polling for message of type {}, returning None",
                        std::any::type_name::<M>()
                    );
                    return Poll::Ready(Ok(None));
                }
                Some(msg) => match M::from_message(msg) {
                    Ok(m) => {
                        log::trace!(
                            "Rx: Parsed message of type {}, returning it",
                            std::any::type_name::<M>()
                        );
                        return Poll::Ready(Ok(Some(m)));
                    }
                    Err(msg) => {
                        log::trace!(
                            "Rx: Message of type {} could not be converted, pushing back to buffer: {:?}",
                            std::any::type_name::<M>(),
                            msg
                        );
                        self.msgs.push(msg);
                    }
                },
            }
        }
    }
    fn close(&mut self) -> (Vec<u8>, Vec<Message>) {
        let Self { reader, msgs } = self;
        (reader.close(), msgs.close())
    }
}

pub struct Rx<R>(Arc<Mutex<RxLocal<R>>>)
where
    R: AsyncRead + Unpin + Send + Sync;
impl<R> Unpin for Rx<R> where R: AsyncRead + Unpin + Send + Sync {}
impl<R> Clone for Rx<R>
where
    R: AsyncRead + Unpin + Send + Sync,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<R> Rx<R>
where
    R: AsyncRead + Unpin + Send + Sync,
{
    pub fn new(reader: R) -> Self {
        Self(Arc::new(Mutex::new(RxLocal::new(reader))))
    }
    pub fn poll_recv<M: FromMessage>(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<Option<M>>> {
        let Ok(mut guard) = self.0.lock() else {
            return Poll::Ready(Err(anyhow!("BUG: Rx Mutex broken")));
        };
        Pin::new(guard.deref_mut()).poll_recv(cx)
    }
    pub fn recv<M: FromMessage>(&mut self) -> Recv<R, M> {
        Recv {
            rx: self.clone(),
            _marker: std::marker::PhantomData,
        }
    }
    pub fn close(&mut self) -> Result<(Vec<u8>, Vec<Message>)> {
        let mut guard = self.0.lock().map_err(|_| anyhow!("BUG: Rx Mutex broken"))?;
        Ok(guard.deref_mut().close())
    }
}

pub struct Recv<R, M>
where
    R: AsyncRead + Unpin + Send + Sync,
    M: FromMessage,
{
    rx: Rx<R>,
    _marker: std::marker::PhantomData<M>,
}
impl<R: Unpin, M> Unpin for Recv<R, M>
where
    R: AsyncRead + Unpin + Send + Sync,
    M: FromMessage,
{
}

impl<R, M> Future for Recv<R, M>
where
    R: AsyncRead + Unpin + Send + Sync,
    M: FromMessage,
{
    type Output = Result<Option<M>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.rx).poll_recv(cx)
    }
}
