mod tx;
pub use tx::Sending;
pub use tx::Tx;
mod rx;
pub use rx::Rx;

#[cfg(test)]
use crate::Message;
#[cfg(test)]
pub(crate) async fn serialize_messages(msgs: impl IntoIterator<Item = &Message>) -> Vec<u8> {
    let mut tx = Tx::new(Vec::new());
    tx.send(msgs).await.expect("BUG: failed to send messages");
    tx.flush().await.expect("BUG: failed to flush Tx");
    tx.close().await.expect("BUG: failed to close Tx")
}

#[cfg(test)]
pub(crate) async fn deserialize_messages(mut input: &[u8]) -> Vec<Message> {
    let mut rx = Rx::new(&mut input);
    let mut res = vec![];
    while let Some(crate::AnyMessage(msg)) = rx.recv().await.expect("Failed to receive message") {
        res.push(msg);
    }
    let (input, skipped) = rx.close().expect("Close failed");
    if !input.is_empty() {
        panic!(
            "Expected all input to be consumed, but {} bytes remain: {:?}",
            input.len(),
            String::from_utf8_lossy(&input)
        );
    }
    if !skipped.is_empty() {
        panic!(
            "Expected no messages to be skipped, but {} were: {:?}",
            skipped.len(),
            skipped
        );
    }
    res
}

mod arc_err;
use arc_err::ArcErr;
