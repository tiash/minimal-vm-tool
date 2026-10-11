use expect_test::expect;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_util::bytes::Bytes;

macro_rules! run {
  ([$e:expr]) => {
    run(dedent::dedent!($e))
  };
}

async fn run(stdin: &str) -> String {
  let mut proc = tokio::process::Command::new(
    std::env::var("CARGO_BIN_EXE_minimal-vm-exec-agent")
      .expect("minimal-vm-exec-agent unavailble."),
  )
  .stdin(std::process::Stdio::piped())
  .stdout(std::process::Stdio::piped())
  .stderr(std::process::Stdio::inherit())
  .spawn()
  .expect("failed to run");

  let stdin = {
    let mut fd = proc.stdin.take().expect("failed to take stdin");
    async move {
      let mut buf = Bytes::copy_from_slice(stdin.as_bytes());
      fd.write_all_buf(&mut buf)
        .await
        .expect("failed to write stdin");
      fd.shutdown().await.expect("failed to shutdown stdin");
      assert!(buf.is_empty(), "stdin not fully consumed");
    }
  };
  let stdout = {
    let mut fd = proc.stdout.take().expect("failed to take stdout");
    async move {
      let mut buf = Vec::new();
      fd.read_to_end(&mut buf)
        .await
        .expect("failed to read stdout");
      buf
    }
  };

  let ((), stdout, exit) = tokio::join!(stdin, stdout, proc.wait(),);
  exit.expect("failed to wait for process");

  String::from_utf8(stdout)
    .expect("malformed utf8")
    .std_cleanup()
}

trait StringExt<'l>: 'l + Sized {
  fn replace_all(
    self,
    patterns_and_replacements: impl IntoIterator<Item = (&'l str, &'l str)>,
  ) -> String;
  fn std_cleanup(self) -> String {
    self.replace_all([
      // mask pid
      (r#""pid":\d+"#, r#""pid":<PID>"#),
      // remove stderr closed message
      (r#"\{"stderr":\{"closed":true\}\}\n"#, r#""#),
      // move exit status to the end
      (r#"(\{"exited":\{"status":\d+\}\}\n)((.*\n)*)$"#, r#"$2$1"#),
    ])
  }
}
impl<'l> StringExt<'l> for &'l str {
  fn replace_all(
    self,
    patterns_and_replacements: impl IntoIterator<Item = (&'l str, &'l str)>,
  ) -> String {
    let mut res = self.to_string();
    for (pattern, replacement) in patterns_and_replacements {
      let re = regex::Regex::new(pattern).unwrap();
      res = re.replace_all(&res, replacement).to_string();
    }
    res
  }
}

#[tokio::test]
async fn echo() {
  simple_test_logging::init();

  let output = run!([r#"
        {"exec":{"prog":"echo","args":["Hello, world!"]}}
    "#])
  .await
  .replace_all([
    // hide stdin closed message
    (r#"\{"stdin":\{"closed":true\}\}\n"#, r#""#),
  ]);

  expect!([r#"
        {"started":{"pid":<PID>}}
        {"stdout":{"data":"Hello, world!\n"}}
        {"stdout":{"closed":true}}
        {"exited":{"status":0}}
    "#])
  .assert_eq(&output);
}

#[tokio::test]
async fn cat() {
  simple_test_logging::init();
  let output = run!([r#"
        {"exec":{"prog":"cat"}}
        {"stdin":{"data":"Hello, world!\n"}}
        {"stdin":{"data":"Good bye, world!\n"}}
        {"stdin":{"closed":true}}
    "#])
  .await;

  expect!([r#"
        {"started":{"pid":<PID>}}
        {"stdin":{"closed":true}}
        {"stdout":{"data":"Hello, world!\n"}}
        {"stdout":{"data":"Good bye, world!\n"}}
        {"stdout":{"closed":true}}
        {"exited":{"status":0}}
    "#])
  .assert_eq(&output);
}

#[tokio::test]
async fn kill() {
  simple_test_logging::init();
  let output = run!([r#"
        {"exec":{"prog":"sleep","args":["1000"]}}
        {"kill":{"signal":15}}
    "#])
  .await
  .replace_all([
    // hide stdin closed message
    (r#"\{"stdin":\{"closed":true\}\}\n"#, r#""#),
  ]);

  expect!([r#"
        {"started":{"pid":<PID>}}
        {"stdout":{"closed":true}}
        {"exited":{"status":15}}
    "#])
  .assert_eq(&output);
}

/// Many agent instances running at the same time must not interfere with each
/// other: each connection is a fully independent agent (its own child process,
/// its own stdio), which is what lets a host fire concurrent exec calls at one
/// VM. Each instance here runs an interactive `cat` (it only produces output
/// once its stdin closes), so all N instances overlap in time; each output must
/// still be exactly its own echo, with no cross-talk.
#[tokio::test]
async fn concurrent_instances_are_independent() {
  simple_test_logging::init();

  const N: usize = 16;

  let one = |i: usize| {
    let stdin = [
      r#"{"exec":{"prog":"cat"}}"#.to_string(),
      format!(r#"{{"stdin":{{"data":"ping {i}\n"}}}}"#),
      r#"{"stdin":{"closed":true}}"#.to_string(),
    ]
    .join("\n");
    let stdin = Bytes::copy_from_slice(stdin.as_bytes());
    async move {
      let mut proc = tokio::process::Command::new(
        std::env::var("CARGO_BIN_EXE_minimal-vm-exec-agent")
          .expect("minimal-vm-exec-agent unavailable."),
      )
      .stdin(std::process::Stdio::piped())
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::inherit())
      .spawn()
      .expect("failed to run");

      let mut stdin_fd = proc.stdin.take().expect("failed to take stdin");
      let mut stdout_fd = proc.stdout.take().expect("failed to take stdout");
      let write = async move {
        stdin_fd.write_all(&stdin).await.unwrap();
        stdin_fd.shutdown().await.unwrap();
      };
      let read = async move {
        let mut buf = Vec::new();
        stdout_fd.read_to_end(&mut buf).await.unwrap();
        buf
      };
      let ((), stdout, status) = tokio::join!(write, read, proc.wait());
      status.expect("agent exited cleanly");
      stdout
    }
  };

  let outputs = futures_in_order(0..N, one).await;
  for (i, out) in outputs.iter().enumerate() {
    let out = String::from_utf8_lossy(out);
    let out = out.replace_all([
      // hide stdin closed message
      (r#"\{"stdin":\{"closed":true\}\}\n"#, r#""#),
    ]);
    let out = out.std_cleanup();
    let out = out.replace(&format!("ping {i}"), "ping 0");
    expect!([r#"
        {"started":{"pid":<PID>}}
        {"stdout":{"data":"ping 0\n"}}
        {"stdout":{"closed":true}}
        {"exited":{"status":0}}
    "#])
    .assert_eq(&out);
  }
}

/// Drive `n` instance futures concurrently and collect their outputs in input
/// order, mirroring a host firing N exec calls at one VM and awaiting them all.
async fn futures_in_order<I, F, T>(inputs: I, f: impl Fn(usize) -> F) -> Vec<T>
where
  I: Iterator<Item = usize>,
  F: std::future::Future<Output = T> + Send + 'static,
  T: Send + 'static,
{
  let mut tasks = Vec::new();
  for i in inputs {
    tasks.push(tokio::spawn(f(i)));
  }
  let mut out = Vec::with_capacity(tasks.len());
  for t in tasks {
    out.push(t.await.expect("instance task panicked"));
  }
  out
}
