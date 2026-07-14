use std::{
  ops::{Deref, DerefMut},
  path::PathBuf,
  process::ExitStatus,
  sync::Arc,
};

use minimal_vm_exec_protocol as protocol;
use tokio::sync::Mutex;

use anyhow::Context as _;
use anyhow::Result;
use anyhow::anyhow;

pub struct Process {
  pid: u32,
  proc: Arc<
    Mutex<
      tokio_util::either::Either<tokio::process::Child, std::result::Result<ExitStatus, ArcErr>>,
    >,
  >,
}

#[derive(Clone)]
struct ArcErr(Arc<anyhow::Error>);
impl ArcErr {
  fn new(err: anyhow::Error) -> Self {
    Self(Arc::new(err))
  }
}
impl std::fmt::Display for ArcErr {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    std::fmt::Display::fmt(&self.0, f)
  }
}
impl std::fmt::Debug for ArcErr {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    std::fmt::Debug::fmt(&self.0, f)
  }
}
impl std::error::Error for ArcErr {}

fn signal_pid(pid: u32, signal: i32) {
  let ok = nix::sys::signal::kill(
    nix::unistd::Pid::from_raw(pid as i32),
    Some(nix::sys::signal::Signal::try_from(signal).unwrap_or(nix::sys::signal::Signal::SIGKILL)),
  );
  log::info!("Sent signal {} to process {}: {:?}", signal, pid, ok);
}
impl Process {
  pub async fn signal(&self, signal: i32) {
    match self.proc.try_lock() {
      | Err(_) => {
        log::error!("Failed to acquire lock to send signal, process is proably running!");
        signal_pid(self.pid, signal);
      },
      | Ok(guard) => match guard.deref() {
        | tokio_util::either::Either::Right(_) => {
          log::error!("Process already exited, not sending signal");
        },
        | tokio_util::either::Either::Left(child) => match child.id() {
          | Some(pid) => {
            signal_pid(pid, signal);
          },
          | None => {
            log::error!("Processs exited recently not sending signal");
          },
        },
      },
    }
  }
  pub async fn wait(&self) -> Result<ExitStatus> {
    let mut guard = self.proc.lock().await;
    Ok(
      (match guard.deref_mut() {
        | tokio_util::either::Either::Left(child) => {
          let exit_status = child
            .wait()
            .await
            .context("Failed to wait for child process")
            .map_err(ArcErr::new);
          *guard = tokio_util::either::Either::Right(exit_status.clone());
          exit_status
        },
        | tokio_util::either::Either::Right(result) => result.clone(),
      })?,
    )
  }
}
impl Drop for Process {
  fn drop(&mut self) {
    if let Ok(mut guard) = self.proc.try_lock() {
      match guard.deref_mut() {
        | tokio_util::either::Either::Left(child) => {
          if let Some(pid) = child.id() {
            signal_pid(pid, 9);
          }
          *guard = tokio_util::either::Either::Right(Err(ArcErr::new(anyhow!(
            "Process killed due to Process handle being dropped"
          ))));
        },
        | tokio_util::either::Either::Right(_) => {},
      }
    } else {
      signal_pid(self.pid, 9);
    }
  }
}

pub struct Child {
  pub pid: u32,
  pub process: Process,
  pub stdin: tokio::process::ChildStdin,
  pub stdout: tokio::process::ChildStdout,
  pub stderr: tokio::process::ChildStderr,
}

pub async fn spawn(exec: &protocol::Exec) -> Result<Child> {
  log::info!(
    "Spawning process: prog={}, args={:?}, pwd={:?}, user={:?}",
    exec.prog,
    exec.args,
    exec.pwd,
    exec.user
  );

  let pwd = match &exec.pwd {
    | Some(p) => PathBuf::from(p),
    | None => std::env::current_dir().context("Failed to get current directory")?,
  };

  let selected_user = {
    if nix::unistd::getuid().is_root() {
      match &exec.user {
        | None => {
          Some(nix::unistd::User::from_name("user")?.ok_or(anyhow!("unknown user: {:?}", "user"))?)
        },
        | Some(user) => {
          Some(nix::unistd::User::from_name(user)?.ok_or(anyhow!("unknown user: {:?}", user))?)
        },
      }
    } else {
      None
    }
  };

  let mut command = if let Some(ref target_user) = selected_user
    .clone()
    .filter(|u| u.uid != nix::unistd::getuid())
  {
    // Drop privileges to the target user AND set up their environment like a
    // login shell. Run the command through the user's login shell
    // (`bash -l -c '...'`), which sources /etc/profile -> /etc/set-environment
    // -> /etc/profile.d/*, giving the child HOME, PATH, LANG, proxy vars, etc.
    // Without this the socket-activated agent's bare env propagates, missing
    // everything a login shell provides.
    let mut c = tokio::process::Command::new("bash");
    c.arg("-l");
    c.arg("-c");
    // Use exec so the shell replaces itself with the target program (the
    // child PID is the program, not bash). The first arg after the `-c`
    // script is $0 (the shell name, excluded from "$@"), so it must be a
    // throwaway — exec.prog has to land at $1 to be part of "$@". The `--`
    // stops exec from treating a leading-dash program/arg (e.g. `bash -c`,
    // `uname -sno`) as its own options.
    c.arg("exec -- \"$@\"");
    c.arg("bash");
    c.arg(&exec.prog);
    c.args(&exec.args);
    // Set HOME/USER so the shell and profile scripts resolve correctly
    // before /etc/set-environment runs.
    c.env("HOME", &target_user.dir);
    c.env("USER", &target_user.name);
    c
  } else {
    // Running as root (or already the target user): direct exec, no shell.
    let mut c = tokio::process::Command::new(&exec.prog);
    c.args(&exec.args);
    c
  };
  command.current_dir(pwd);

  command.stdin(std::process::Stdio::piped());
  command.stdout(std::process::Stdio::piped());
  command.stderr(std::process::Stdio::piped());

  if let Some(target_user) = selected_user
    .clone()
    .filter(|u| u.uid != nix::unistd::getuid())
  {
    log::info!("Dropping privileges to user: {}", target_user.name);
    // SAFETY: This closure runs in the child process between fork and
    // exec (pre_exec). The functions called (setgroups, setgid, setuid)
    // are POSIX async-signal-safe. We clear supplementary groups BEFORE
    // setgid to prevent retaining root's supplementary groups (H4).
    // Order matters: setgroups -> setgid -> setuid (never reverse).
    unsafe {
      command.pre_exec(move || {
        // setgroups is only available on Linux (nix gates it out on
        // apple_targets). The exec-agent runs inside the Linux VM.
        #[cfg(target_os = "linux")]
        if let Err(e) = nix::unistd::setgroups(&[]) {
          return Err(std::io::Error::other(format!("setgroups failed: {}", e)));
        }
        if let Err(e) = nix::unistd::setgid(target_user.gid) {
          return Err(std::io::Error::other(format!("setgid failed: {}", e)));
        }
        if let Err(e) = nix::unistd::setuid(target_user.uid) {
          return Err(std::io::Error::other(format!("setuid failed: {}", e)));
        }
        Ok(())
      });
    };
  }

  let mut child = command.spawn().context("Failed to spawn child process")?;
  let pid = child.id().unwrap_or(0);
  log::info!("Process spawned with PID: {}", pid);

  let stdin = child
    .stdin
    .take()
    .ok_or(anyhow::anyhow!("Failed to capture child stdin"))?;
  let stdout = child
    .stdout
    .take()
    .ok_or(anyhow::anyhow!("Failed to capture child stdout"))?;
  let stderr = child
    .stderr
    .take()
    .ok_or(anyhow::anyhow!("Failed to capture child stderr"))?;

  Ok(Child {
    pid,
    stdin,
    stdout,
    stderr,
    process: Process {
      pid,
      proc: Arc::new(Mutex::new(tokio_util::either::Either::Left(child))),
    },
  })
}
