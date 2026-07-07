use tokio_util::bytes::Bytes;
use tokio_util::bytes::BytesMut;

use serde::{Deserializer, Serialize as _, Serializer};
pub fn serialize<S>(data: &Bytes, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if data.is_ascii() {
        serializer.serialize_str(std::str::from_utf8(data).unwrap())
    } else {
        data.serialize(serializer)
    }
}
pub fn deserialize<'de, D>(deserializer: D) -> Result<Bytes, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(Visitor)
}
struct Visitor;
impl<'de> serde::de::Visitor<'de> for Visitor {
    type Value = Bytes;
    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a byte array or a UTF-8 string")
    }
    fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Bytes::copy_from_slice(v))
    }
    fn visit_borrowed_bytes<E>(self, v: &'de [u8]) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Bytes::copy_from_slice(v))
    }
    fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Bytes::from(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Bytes::copy_from_slice(v.as_bytes()))
    }
    fn visit_borrowed_str<E>(self, v: &'de str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(Bytes::copy_from_slice(v.as_bytes()))
    }
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let mut buf = BytesMut::with_capacity(seq.size_hint().unwrap_or(1024));

        while let Some(byte) = seq.next_element()? {
            buf.extend_from_slice(&[byte]);
        }
        Ok(buf.into())
    }
}

#[cfg(test)]
use quickcheck::Arbitrary as _;

#[cfg(test)]
pub fn arbitrary(g: &mut quickcheck::Gen) -> Bytes {
    let size = usize::arbitrary(g) % 1024;
    let mut buf = vec![0; size];
    for byte in &mut buf {
        *byte = u8::arbitrary(g);
    }
    Bytes::from(buf)
}
