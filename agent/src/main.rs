use anyhow::Result;
use anyhow::anyhow;
use clap::Parser;
use minimal_vm_exec_protocol as protocol;
use std::os::{fd::AsRawFd, unix::prelude::ExitStatusExt as _};
use tokio::{
  io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _},
  select,
};
use tokio_util::bytes::Bytes;

/// Command-line arguments for the minimal-vm-exec-agent.
#[derive(clap::Parser, Debug)]
#[command(name = "minimal-vm-exec-agent")]
#[command(
  about = "Minimal VM Exec Agent",
  long_about = "This agent is intended to be launched by inetd or systemd (with inetd calling conventions) for each connection to the exec protocol. It expects to get the virtio-vsock connection on its stdin/stdout. It will then listen for JSON messages according to the specification and execute one process inside the VM accordingly."
)]
struct Args {
  /// Increase logging verbosity above the default (`Info`).
  /// `-v` selects `Debug`, `-vv` selects `Trace`.
  #[arg(short, long, action = clap::ArgAction::Count)]
  verbose: u8,

  /// Decrease logging verbosity below the default (`Info`).
  /// `-q` selects `Warn`, `-qq` selects `Error`.
  #[arg(short, long, action = clap::ArgAction::Count)]
  quiet: u8,
}

/// Resolve the `-v` / `-q` flag counts into a `stderrlog` verbosity level.
fn verbosity_from_flags(verbose: u8, quiet: u8) -> stderrlog::LogLevelNum {
  match verbose as i16 - quiet as i16 {
    | n if n <= -2 => stderrlog::LogLevelNum::Error,
    | -1 => stderrlog::LogLevelNum::Warn,
    | 0 => stderrlog::LogLevelNum::Info,
    | 1 => stderrlog::LogLevelNum::Debug,
    | _ => stderrlog::LogLevelNum::Trace,
  }
}

#[tokio::main]
async fn main() {
  let args = Args::parse();

  stderrlog::new()
    .verbosity(verbosity_from_flags(args.verbose, args.quiet))
    .timestamp(stderrlog::Timestamp::Millisecond)
    .color(stderrlog::ColorChoice::Auto)
    .show_module_names(true)
    .init()
    .expect("Failed to initialize logger");

  log::info!("Minimal VM Exec Agent starting with args: {:?}", args);

  log::info!("Minimal VM Exec Agent starting...");

  let mut tx = protocol::io::Tx::new(tokio::io::stdout());

  let status: Result<()> = async {
        let mut rx = protocol::io::Rx::new(tokio::io::BufReader::new(tokio::io::stdin()));

    let exec : protocol::Exec = match rx.recv().await? {
                Some(exec) => Ok(exec),
                    None => Err(anyhow!("Connection closed")),
    }?;
        let line_buffered = exec.line_buffered_io.unwrap_or(true);


    log::info!("Received exec message: {:?}", exec);
    let process::Child {
        pid,
        process,
        stdin: mut child_stdin,
        stdout: mut child_stdout,
        stderr: mut child_stderr,
    } = process::spawn(&exec).await.map_err(|e| anyhow!("Failed to spawn child {:?}", e))?;


    tx.send(&[protocol::Message::Started(protocol::Started {
        pid,
    })])
    .await?;


    let stdin = {
    let rx = rx.clone();
    let tx = tx.clone();
        async move || -> Result<()> {
            let mut rx = rx;
            let mut tx = tx;
            log::debug!("Stdin copy task starting...");
                while let Some(protocol::Stdin(Some(data))) = rx.recv::<protocol::Stdin<Option<Bytes>>>().await? {
                    log::debug!("Received {} bytes of stdin data from protocol", data.len());
                    log::debug!("Writing {} bytes to child stdin", data.len());
                    child_stdin.write_all(&data).await?;
                }
            log::debug!("Stdin copy data end...");
            tx.send(&[protocol::Message::Stdin(protocol::Stdio::Closed)]).await?;
            log::debug!("Stdin copy task exiting...");
            Ok(())
    }};
    let stdout = {
    let rx = rx.clone();
    let tx = tx.clone();
        async move || -> Result<()> {
            let mut rx = rx;
            let mut tx = tx;
            let mut buffer = Vec::new();
            loop {
                select!{
            data = read_line_or_chunk(&mut child_stdout, line_buffered, &mut buffer) => {
                if let Some(data) = data? {
                    log::debug!("Sending {} bytes on stdout", data.len());
                    tx.send(&[protocol::Message::Stdout(protocol::Stdio::Data(data))]).await?;
                }else {
                    break;
                }
            },
            Ok(Some(protocol::Stdout(protocol::Stdio::Closed))) = rx.recv() => {
                nix::unistd::close(child_stdout.as_raw_fd()).map_err(|e| anyhow!("closing stdout: {:?}", e))?;
            }
        }
    }
    tx.send(&[protocol::Message::Stdout(protocol::Stdio::Closed)]).await?;
    Ok(())
}};
let stderr = {
    let rx = rx.clone();
    let tx = tx.clone();
        async move || -> Result<()> {
            let mut rx = rx;
            let mut tx = tx;
            let mut buffer = Vec::new();
            loop {
                select!{
            data = read_line_or_chunk(&mut child_stderr, line_buffered, &mut buffer) =>  {
                if let Some(data) = data? {
                    log::debug!("Sending {} bytes on stderr", data.len());
                    tx.send(&[protocol::Message::Stderr(protocol::Stdio::Data(data))]).await?;
                }else {
                    break;
                }
            },
            Ok(Some(protocol::Stderr(protocol::Stdio::Closed))) = rx.recv() => {
                nix::unistd::close(child_stderr.as_raw_fd()).map_err(|e| anyhow!("closing stderr: {:?}", e))?;
            }
        }
    }
    tx.send(&[protocol::Message::Stderr(protocol::Stdio::Closed)]).await?;
    Ok(())
}};
    let exit = {
    let rx = rx.clone();
    let tx = tx.clone();
        async move || -> Result<()> {
            let mut rx=rx;
            let mut tx=tx;
        log::debug!("Kill signal task starting...");
        loop {
       select!{
        kill = rx.recv() => {
            if let Some(protocol::Kill{signal}) = kill? {
                log::debug!("Received kill signal: {}", signal);
                process.signal(signal).await;
            }
        },
        exit = process.wait() => {
            log::debug!("Process exited with status: {:?}", exit);
            tx.send(&[protocol::Message::Exited(protocol::Exited {
                status: exit?.into_raw(),
            })]).await?;
            break;
         },
    }}
    log::debug!("Kill signal task exiting...");
    Ok(())
}};


    log::debug!("Async jobs starting, Waiting for process to exit...");

   let (exit,stdin,stdout,stderr) = tokio::join!(biased; exit(), stdin(), stdout(), stderr());
    log::debug!("All tasks completed, waiting for connection to close...");
    exit?;
    stdin?;
    stdout?;
    stderr?;
    Ok(())}.await;
  log::info!("Main task completed, status: {:?}", status);
  match &status {
    | Ok(()) => {},
    | Err(e) => {
      let _: Result<()> = tx
        .send(&[protocol::Message::Error(protocol::Error {
          message: e.to_string(),
        })])
        .await;
    },
  }
  log::debug!("closing stdin");
  let _ = nix::unistd::close(nix::libc::STDIN_FILENO);
  log::debug!("closing stdout");
  let _: Result<()> = tx.flush().await;
  if let Ok(mut stdout) = tx.close().await {
    let _: std::io::Result<()> = stdout.flush().await;
    let _: std::io::Result<()> = stdout.shutdown().await;
    log::debug!("closing stdout done");
  }
  // let _ = nix::unistd::close(nix::libc::STDOUT_FILENO);
  // let _ = nix::unistd::close(nix::libc::STDERR_FILENO);

  //unsafe {
  //nix::libc::exit(0);
  //}
  log::info!("Minimal VM Exec Agent exiting...");
}

mod process;

async fn read_line_or_chunk(
  mut r: impl AsyncRead + Unpin,
  line_buffered: bool,
  buffer: &mut Vec<u8>,
) -> Result<Option<Bytes>, std::io::Error> {
  if line_buffered {
    loop {
      if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
        let data = Bytes::copy_from_slice(&buffer[..=pos]);
        buffer.drain(..=pos);
        return Ok(Some(data));
      } else {
        if r.read_buf(buffer).await? == 0 {
          if buffer.is_empty() {
            return Ok(None);
          } else {
            let data = Bytes::copy_from_slice(buffer);
            buffer.clear();
            return Ok(Some(data));
          }
        }
      }
    }
  } else {
    match r.read_buf(buffer).await? {
      | 0 if buffer.is_empty() => Ok(None),
      | _ => {
        let data = Bytes::copy_from_slice(buffer);
        buffer.clear();
        Ok(Some(data))
      },
    }
  }
}
