pub const PORT: u32 = u32::from_be_bytes(*b"exec");

#[test]
fn show_port() {
  expect_test::expect![[r#"1702389091"#]].assert_eq(&format!("{}", PORT));
}

pub mod io;

pub trait FromMessage: Sized {
  fn from_message(msg: Message) -> Result<Self, Message>;
}

#[cfg(test)]
use derive_quickcheck_arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use strum::EnumDiscriminants;
#[cfg(test)]
use strum::VariantArray;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(Arbitrary))]
pub struct Exec {
  pub prog: String,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub args: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pwd: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub user: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub line_buffered_io: Option<bool>,
}
impl FromMessage for Exec {
  fn from_message(msg: Message) -> Result<Self, Message> {
    match msg {
      | Message::Exec(exec) => Ok(exec),
      | msg => Err(msg),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(Arbitrary))]
pub struct Started {
  pub pid: u32,
}
impl FromMessage for Started {
  fn from_message(msg: Message) -> Result<Self, Message> {
    match msg {
      | Message::Started(started) => Ok(started),
      | msg => Err(msg),
    }
  }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(EnumDiscriminants, Arbitrary))]
enum StdioTrue {
  True,
}
impl Serialize for StdioTrue {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: serde::Serializer,
  {
    serializer.serialize_bool(true)
  }
}
impl<'a> Deserialize<'a> for StdioTrue {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: serde::Deserializer<'a>,
  {
    match bool::deserialize(deserializer)? {
      | true => Ok(StdioTrue::True),
      | false => Err(serde::de::Error::custom("Expected true")),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(EnumDiscriminants, Arbitrary))]
#[cfg_attr(test, strum_discriminants(derive(VariantArray, Arbitrary)))]
#[cfg_attr(test, strum_discriminants(vis(pub(crate))))]
pub enum Stdio {
  #[serde(rename = "data", with = "data_bytes")]
  Data(#[cfg_attr(test, arbitrary(gen(data_bytes::arbitrary)))] tokio_util::bytes::Bytes),
  #[serde(rename = "closed")]
  #[allow(private_interfaces, non_camel_case_types)]
  Priv__Closed(StdioTrue),
}
impl Stdio {
  #[allow(non_upper_case_globals)]
  pub const Closed: Self = Self::Priv__Closed(StdioTrue::True);
}
#[cfg(test)]
impl StdioDiscriminants {
  #[allow(non_upper_case_globals)]
  pub(crate) const Closed: Self = Self::Priv__Closed;
}
mod data_bytes;

pub struct Closed;

macro_rules! fromStdioMessage {
  ($NAME:ident) => {
    pub struct $NAME<T>(pub T);

    impl FromMessage for $NAME<Stdio> {
      fn from_message(msg: Message) -> Result<Self, Message> {
        match msg {
          | Message::$NAME(stdio) => Ok($NAME(stdio)),
          | msg => Err(msg),
        }
      }
    }
    impl FromMessage for $NAME<Closed> {
      fn from_message(msg: Message) -> Result<Self, Message> {
        match msg {
          | Message::$NAME(Stdio::Closed) => Ok($NAME(Closed)),
          | msg => Err(msg),
        }
      }
    }
    impl FromMessage for $NAME<tokio_util::bytes::Bytes> {
      fn from_message(msg: Message) -> Result<Self, Message> {
        match msg {
          | Message::$NAME(Stdio::Data(bytes)) => Ok($NAME(bytes)),
          | msg => Err(msg),
        }
      }
    }
    impl FromMessage for $NAME<Option<tokio_util::bytes::Bytes>> {
      fn from_message(msg: Message) -> Result<Self, Message> {
        match msg {
          | Message::$NAME(Stdio::Data(bytes)) => Ok($NAME(Some(bytes))),
          | Message::$NAME(Stdio::Closed) => Ok($NAME(None)),
          | msg => Err(msg),
        }
      }
    }
  };
}

fromStdioMessage!(Stdin);
fromStdioMessage!(Stdout);
fromStdioMessage!(Stderr);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(Arbitrary))]
pub struct Exited {
  pub status: i32,
}
impl FromMessage for Exited {
  fn from_message(msg: Message) -> Result<Self, Message> {
    match msg {
      | Message::Exited(exited) => Ok(exited),
      | msg => Err(msg),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(Arbitrary))]
pub struct Kill {
  pub signal: i32,
}
impl FromMessage for Kill {
  fn from_message(msg: Message) -> Result<Self, Message> {
    match msg {
      | Message::Kill(kill) => Ok(kill),
      | msg => Err(msg),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(Arbitrary))]
pub struct Error {
  pub message: String,
}
impl std::fmt::Display for Error {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let Self { message } = self;
    f.write_str(message)
  }
}
impl FromMessage for Error {
  fn from_message(msg: Message) -> Result<Self, Message> {
    match msg {
      | Message::Error(err) => Ok(err),
      | msg => Err(msg),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(Arbitrary))]
struct NoOp;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(test, derive(EnumDiscriminants, Arbitrary))]
#[cfg_attr(test, strum_discriminants(derive(VariantArray, Arbitrary)))]
#[cfg_attr(test, strum_discriminants(vis(pub(crate))))]
pub enum Message {
  #[serde(rename = "exec")]
  Exec(Exec),
  #[serde(rename = "started")]
  #[allow(private_interfaces, non_camel_case_types)]
  Started(Started),
  #[serde(rename = "stdin")]
  Stdin(Stdio),
  #[serde(rename = "stdout")]
  Stdout(Stdio),
  #[serde(rename = "stderr")]
  Stderr(Stdio),
  #[serde(rename = "kill")]
  #[allow(private_interfaces, non_camel_case_types)]
  Kill(Kill),
  #[serde(rename = "exited")]
  Exited(Exited),
  #[serde(rename = "error")]
  Error(Error),
  #[serde(rename = "private-noop")]
  #[allow(private_interfaces, non_camel_case_types)]
  #[cfg_attr(test, arbitrary(skip))]
  Priv__NoOp(NoOp),
}
impl Message {
  #[allow(non_upper_case_globals)]
  pub(crate) const NoOp: Self = Self::Priv__NoOp(NoOp);
}

#[cfg(test)]
pub struct AnyMessage(pub Message);
#[cfg(test)]
impl FromMessage for AnyMessage {
  fn from_message(msg: Message) -> Result<Self, Message> {
    Ok(Self(msg))
  }
}

#[cfg(test)]
impl MessageDiscriminants {
  #[allow(non_upper_case_globals)]
  pub(crate) const NoOp: Self = Self::Priv__NoOp;
}

#[cfg(test)]
mod tests;
