use crate::io::{deserialize_messages, serialize_messages};
use crate::*;
use quickcheck_macros::quickcheck;
use strum::VariantArray;
use tokio_util::bytes::Bytes;

fn messages() -> impl Iterator<Item = Message> {
    MessageDiscriminants::VARIANTS
        .iter()
        .flat_map(|verb| match *verb {
            MessageDiscriminants::Exec => vec![
                Message::Exec(Exec {
                    prog: "echo".to_string(),
                    args: vec!["hello".to_string()],
                    pwd: Some("/tmp".to_string()),
                    user: Some("user".to_string()),
                    line_buffered_io: Some(true),
                }),
                Message::Exec(Exec {
                    prog: "echo".to_string(),
                    args: vec!["bye".to_string()],
                    pwd: None,
                    user: None,
                    line_buffered_io: None,
                }),
            ],

            MessageDiscriminants::Started => vec![Message::Started(Started { pid: 123 })],
            MessageDiscriminants::Stdin => StdioDiscriminants::VARIANTS
                .iter()
                .map(|subverb| match *subverb {
                    StdioDiscriminants::Data => {
                        Stdio::Data(Bytes::from_static(b"input to stdin\n"))
                    }
                    StdioDiscriminants::Closed => Stdio::Closed,
                })
                .map(Message::Stdin)
                .collect(),
            MessageDiscriminants::Stdout => StdioDiscriminants::VARIANTS
                .iter()
                .map(|subverb| match *subverb {
                    StdioDiscriminants::Data => {
                        Stdio::Data(Bytes::from_static(b"output to stdout\n"))
                    }
                    StdioDiscriminants::Closed => Stdio::Closed,
                })
                .map(Message::Stdout)
                .collect(),

            MessageDiscriminants::Stderr => StdioDiscriminants::VARIANTS
                .iter()
                .map(|subverb| match *subverb {
                    StdioDiscriminants::Data => {
                        Stdio::Data(Bytes::from_static(b"output to stderr\n"))
                    }
                    StdioDiscriminants::Closed => Stdio::Closed,
                })
                .map(Message::Stderr)
                .collect(),

            MessageDiscriminants::Kill => vec![Message::Kill(Kill { signal: 9 })],
            MessageDiscriminants::Exited => vec![Message::Exited(Exited { status: 0 })],
            MessageDiscriminants::Error => vec![Message::Error(Error {
                message: "test error".to_string(),
            })],
            MessageDiscriminants::NoOp => vec![],
        })
}

#[tokio::test]
async fn roundtrip_single() {
    for msg in messages() {
        let serialized = serialize_messages([&msg]).await;
        let deserialized = deserialize_messages(&serialized).await;
        assert_eq!(vec![msg], deserialized);
    }
}
#[tokio::test]
async fn show_specimen() {
    let messages_bytes = serialize_messages(&messages().collect::<Vec<_>>()).await;
    let messages_string = String::from_utf8(messages_bytes).expect("malformed utf8");
    expect_test::expect![[r#"
        {"exec":{"prog":"echo","args":["hello"],"pwd":"/tmp","user":"user","line_buffered_io":true}}
        {"exec":{"prog":"echo","args":["bye"]}}
        {"started":{"pid":123}}
        {"stdin":{"data":"input to stdin\n"}}
        {"stdin":{"closed":true}}
        {"stdout":{"data":"output to stdout\n"}}
        {"stdout":{"closed":true}}
        {"stderr":{"data":"output to stderr\n"}}
        {"stderr":{"closed":true}}
        {"kill":{"signal":9}}
        {"exited":{"status":0}}
        {"error":{"message":"test error"}}
    "#]]
    .assert_eq(&messages_string);
}
#[tokio::test]
async fn roundtrip_multi() {
    let messages_bytes = serialize_messages(&messages().collect::<Vec<_>>()).await;
    let deserialized_msgs: Vec<Message> = deserialize_messages(&messages_bytes).await;
    assert_eq!(messages().collect::<Vec<_>>(), deserialized_msgs);
}

#[tokio::test]
async fn match_patterns() {
    for msg in messages() {
        match msg {
            Message::Exec(Exec {
                prog: _,
                args: _,
                pwd: _,
                user: _,
                line_buffered_io: _,
            }) => (),
            Message::Started(Started { pid: _ }) => (),
            Message::Stdin(Stdio::Data(_)) => (),
            Message::Stdin(Stdio::Closed) => (),
            Message::Stdout(Stdio::Data(_)) => (),
            Message::Stdout(Stdio::Closed) => (),
            Message::Stderr(Stdio::Data(_)) => (),
            Message::Stderr(Stdio::Closed) => (),
            Message::Kill(Kill { signal: _ }) => (),
            Message::Exited(Exited { status: _ }) => (),
            Message::Error(Error { message: _ }) => (),
            Message::NoOp => (),
        }
    }
}

#[quickcheck]
fn roundtrip_one(msg: Message) {
    tokio::runtime::LocalRuntime::new()
        .unwrap()
        .block_on(async {
            let serialized = serialize_messages([&msg]).await;
            let deserialized = deserialize_messages(&serialized).await;
            assert_eq!(vec![msg], deserialized);
        });
}
#[quickcheck]
fn roundtrip_many(msgs: Vec<Message>) {
    tokio::runtime::LocalRuntime::new()
        .unwrap()
        .block_on(async {
            let serialized = serialize_messages(&msgs).await;
            let deserialized = deserialize_messages(&serialized).await;
            assert_eq!(msgs, deserialized);
        });
}
