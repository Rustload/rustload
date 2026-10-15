//! Child processes: yt-dlp downloads streamed line by line, and one-shot runs (`-J`, `--version`).
//!
//! The official yt-dlp builds are PyInstaller bundles: a small bootloader starts the real Python
//! process as its child. Killing only the process we spawned would leave that child running (and
//! downloading), so stopping always kills the whole tree: the process group on Unix, `taskkill /T`
//! on Windows. That includes processes whose future is dropped before they exit (client gone,
//! outer time limit): see [`Tree`].

use std::{
    ffi::OsString,
    io,
    path::Path,
    process::{ExitStatus, Stdio},
    time::Duration,
};

use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader},
    process::{Child, Command},
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

use super::output::{self, Line};

/// Largest `-J` output accepted (a 500-entry playlist is a few MiB).
const MAX_JSON_OUTPUT: u64 = 64 * 1024 * 1024;
const MAX_STDERR: u64 = 256 * 1024;

/// A command for a tool, with UTF-8 output whatever the system locale. On Unix, `tmp_dir`
/// replaces the system temporary folder: PyInstaller builds unpack themselves there, which fails
/// when `/tmp` is read-only or mounted `noexec`.
#[cfg_attr(not(unix), allow(unused_variables))]
fn command(program: &Path, tmp_dir: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        // Own process group, so that the whole tree can be stopped at once.
        command.process_group(0);
        command.env("TMPDIR", tmp_dir);
    }
    command
}

/// A child process whose whole tree is killed when it is dropped before it exited.
struct Tree {
    child: Child,
    /// Exited and reaped: nothing left to kill (its id may already belong to another process).
    exited: bool,
}

impl Tree {
    fn spawn(command: &mut Command) -> io::Result<Self> {
        Ok(Self { child: command.spawn()?, exited: false })
    }

    async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait().await;
        self.exited = status.is_ok();
        status
    }

    /// Stops the process and everything it started, and waits for it.
    async fn kill(&mut self) {
        #[cfg(unix)]
        kill_group(&self.child);
        #[cfg(windows)]
        if let Some(pid) = self.child.id() {
            let _ = Command::from(taskkill(pid)).status().await;
        }
        let _ = self.child.kill().await;
        self.exited = true;
    }
}

impl Drop for Tree {
    /// Cannot wait here: the tree is signalled and the runtime reaps the process.
    fn drop(&mut self) {
        if self.exited {
            return;
        }
        #[cfg(unix)]
        kill_group(&self.child);
        // `taskkill /F` also kills the process itself; killing it first would hide its children
        // from `/T`. It runs detached.
        #[cfg(windows)]
        if let Some(pid) = self.child.id()
            && taskkill(pid).spawn().is_ok()
        {
            return;
        }
        let _ = self.child.start_kill();
    }
}

/// Kills the process group led by `child` (see [`command`]).
#[cfg(unix)]
fn kill_group(child: &Child) {
    if let Some(pid) = child.id().and_then(|pid| i32::try_from(pid).ok()) {
        use nix::{sys::signal, unistd::Pid};
        let _ = signal::killpg(Pid::from_raw(pid), signal::Signal::SIGKILL);
    }
}

/// Kills process `pid` and its descendants.
#[cfg(windows)]
fn taskkill(pid: u32) -> std::process::Command {
    let mut command = std::process::Command::new("taskkill");
    command
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// A running yt-dlp download whose output arrives as parsed [`Line`]s.
pub struct Running {
    process: Tree,
    lines: mpsc::Receiver<Line>,
}

impl Running {
    pub fn spawn(program: &Path, args: &[OsString], tmp_dir: &Path) -> io::Result<Self> {
        let mut process = Tree::spawn(command(program, tmp_dir).args(args))?;
        let (tx, lines) = mpsc::channel(256);
        if let Some(stdout) = process.child.stdout.take() {
            tokio::spawn(forward_lines(stdout, tx.clone(), output::parse_stdout));
        }
        if let Some(stderr) = process.child.stderr.take() {
            tokio::spawn(forward_lines(stderr, tx, output::parse_stderr));
        }
        Ok(Self { process, lines })
    }

    /// Next output line; `None` once both streams are closed.
    pub async fn next_line(&mut self) -> Option<Line> {
        self.lines.recv().await
    }

    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.process.wait().await
    }

    pub async fn kill(&mut self) {
        self.process.kill().await;
    }
}

async fn forward_lines(stream: impl AsyncRead + Unpin, tx: mpsc::Sender<Line>, parse: fn(&str) -> Line) {
    let mut reader = BufReader::new(stream);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        match reader.read_until(b'\n', &mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                if tx.send(parse(&String::from_utf8_lossy(&buffer))).await.is_err() {
                    return;
                }
            }
        }
    }
}

/// Output of a process run to completion.
pub struct Finished {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Finished {
    /// Last `ERROR:` line, or the last non-empty stderr line.
    pub fn error(&self) -> Option<String> {
        let lines = || self.stderr.lines().map(str::trim).filter(|l| !l.is_empty());
        lines()
            .filter_map(|line| line.strip_prefix("ERROR: "))
            .next_back()
            .map(str::to_owned)
            .or_else(|| lines().next_back().map(str::to_owned))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("cannot start {0}: {1}")]
    Spawn(String, io::Error),
    #[error("no answer after {0} s")]
    Timeout(u64),
    #[error("stopped")]
    Cancelled,
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Runs `program` until it exits, `timeout` elapses or `cancel` fires (the process tree is
/// killed in both cases, as when the returned future is dropped). See [`command`] for `tmp_dir`.
pub async fn run(
    program: &Path,
    args: &[OsString],
    tmp_dir: &Path,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<Finished, RunError> {
    let mut process = Tree::spawn(command(program, tmp_dir).args(args))
        .map_err(|e| RunError::Spawn(program.display().to_string(), e))?;
    let mut stdout = process.child.stdout.take();
    let mut stderr = process.child.stderr.take();
    let outcome = {
        let completion = async {
            let (out, err) =
                tokio::join!(read_all(stdout.as_mut(), MAX_JSON_OUTPUT), read_all(stderr.as_mut(), MAX_STDERR));
            let status = process.wait().await;
            (out, err, status)
        };
        tokio::select! {
            _ = cancel.cancelled() => Err(RunError::Cancelled),
            result = tokio::time::timeout(timeout, completion) => result.map_err(|_| RunError::Timeout(timeout.as_secs())),
        }
    };
    match outcome {
        Ok((stdout, stderr, status)) => {
            Ok(Finished { status: status?, stdout: stdout?, stderr: String::from_utf8_lossy(&stderr?).into_owned() })
        }
        Err(error) => {
            process.kill().await;
            Err(error)
        }
    }
}

async fn read_all(stream: Option<&mut (impl AsyncRead + Unpin)>, limit: u64) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    if let Some(stream) = stream {
        stream.take(limit).read_to_end(&mut data).await?;
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(stderr: &str) -> Finished {
        #[cfg(unix)]
        let status = std::os::unix::process::ExitStatusExt::from_raw(256);
        #[cfg(windows)]
        let status = std::os::windows::process::ExitStatusExt::from_raw(1);
        Finished { status, stdout: Vec::new(), stderr: stderr.into() }
    }

    #[tokio::test]
    async fn dropped_processes_are_killed_with_their_children() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("finished");
        // Like a PyInstaller bundle: the process starts a child that writes the marker after about
        // two seconds, unless it is killed first.
        #[cfg(windows)]
        let (program, args) = {
            let child = dir.path().join("child.cmd");
            std::fs::write(&child, "@ping -n 3 127.0.0.1 > nul\r\n@type nul > \"%~dp0finished\"\r\n").unwrap();
            let parent = dir.path().join("parent.cmd");
            std::fs::write(&parent, "@start /b \"\" \"%~dp0child.cmd\"\r\n@ping -n 6 127.0.0.1 > nul\r\n").unwrap();
            (parent, Vec::new())
        };
        #[cfg(unix)]
        let (program, args) = (
            std::path::PathBuf::from("sh"),
            vec![
                OsString::from("-c"),
                "(sleep 2 && touch \"$1\") & sleep 5".into(),
                "sh".into(),
                marker.clone().into(),
            ],
        );
        let cancel = CancellationToken::new();
        let run = run(&program, &args, dir.path(), Duration::from_secs(30), &cancel);
        assert!(tokio::time::timeout(Duration::from_millis(600), run).await.is_err());
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(!marker.exists());
    }

    #[test]
    fn extracts_the_last_error() {
        let output = finished("WARNING: slow\nERROR: [youtube] a: first\nERROR: [youtube] a: Video unavailable\n");
        assert_eq!(output.error().as_deref(), Some("[youtube] a: Video unavailable"));
        assert_eq!(
            finished("usage: yt-dlp\nyt-dlp: error: no such option: --nope\n").error().as_deref(),
            Some("yt-dlp: error: no such option: --nope")
        );
        assert_eq!(finished("\n").error(), None);
    }
}
