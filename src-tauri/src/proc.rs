//! Waiting on a child process without trusting it to ever finish, or to stop
//! talking: a pathological input or a stalled server must cost a worker
//! thread a bounded time and the process a bounded amount of memory.

use std::io::Read;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Collect up to `max + 1` bytes of a pipe on its own thread, so that neither
/// stream can fill up and block the child while the other is being read.
fn drain<R: Read + Send + 'static>(pipe: Option<R>, max: u64) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(pipe) = pipe {
            let _ = pipe.take(max + 1).read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

/// `Child::wait_with_output` with limits: the child is killed if it runs past
/// `timeout`, and more than `max_output` bytes on either stream is an error
/// rather than a truncated result. Pipes that outlive the child (a background
/// grandchild holding them open) are covered by the same deadline.
pub(crate) fn wait_bounded(
    mut child: Child,
    timeout: Duration,
    max_output: u64,
) -> Result<Output, String> {
    let timed_out = || format!("Timed out after {}s", timeout.as_secs_f32().ceil() as u64);
    // As wait_with_output does: a child reading stdin to EOF would
    // otherwise wait for it until the deadline.
    drop(child.stdin.take());
    let stdout = drain(child.stdout.take(), max_output);
    let stderr = drain(child.stderr.take(), max_output);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(timed_out());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let collect = |rx: mpsc::Receiver<Vec<u8>>| {
        let buf = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| timed_out())?;
        if buf.len() as u64 > max_output {
            return Err(format!("Output exceeds {max_output} bytes"));
        }
        Ok(buf)
    };
    Ok(Output {
        status,
        stdout: collect(stdout)?,
        stderr: collect(stderr)?,
    })
}

/// `Command::output` under `wait_bounded`'s limits, for a child that may
/// never finish (a dead network mount, a stalled server, a crafted file).
/// It gets no stdin.
pub(crate) fn output_bounded(
    cmd: &mut Command,
    timeout: Duration,
    max_output: u64,
) -> Result<Output, String> {
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    wait_bounded(child, timeout, max_output)
}

#[cfg(test)]
mod tests {
    use super::{output_bounded, wait_bounded};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    fn sh(script: &str) -> Child {
        Command::new("/bin/sh")
            .args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[test]
    fn collects_both_streams_and_the_status() {
        let out = wait_bounded(sh("echo out; echo err >&2; exit 3"), Duration::from_secs(10), 1024)
            .unwrap();
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
    }

    #[test]
    fn a_hung_child_is_killed_at_the_deadline() {
        let child = sh("echo $$; exec sleep 30");
        let pid = child.id();
        let t = Instant::now();
        let err = wait_bounded(child, Duration::from_millis(300), 1024).unwrap_err();
        assert!(err.contains("Timed out"), "{err}");
        assert!(t.elapsed() < Duration::from_secs(5));
        // SAFETY: signal 0 only probes whether the pid exists.
        assert_ne!(unsafe { libc::kill(pid as i32, 0) }, 0, "child is gone");
    }

    #[test]
    fn pipes_held_open_by_a_grandchild_cannot_outlast_the_deadline() {
        let t = Instant::now();
        let err = wait_bounded(sh("sleep 30 & exit 0"), Duration::from_millis(300), 1024).unwrap_err();
        assert!(err.contains("Timed out"), "{err}");
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn flooding_output_is_an_error_not_a_truncated_result() {
        let flood = sh("yes | head -c 2000000");
        let err = wait_bounded(flood, Duration::from_secs(10), 1024).unwrap_err();
        assert!(err.contains("exceeds"), "{err}");
    }

    #[test]
    fn stdin_is_closed_so_a_reader_finishes() {
        let child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let out = wait_bounded(child, Duration::from_secs(5), 1024).unwrap();
        assert!(out.status.success());
    }

    #[test]
    fn output_bounded_runs_and_times_out() {
        let out = output_bounded(
            Command::new("/bin/sh").args(["-c", "echo hi"]),
            Duration::from_secs(5),
            1024,
        )
        .unwrap();
        assert_eq!(out.stdout, b"hi\n");
        let t = Instant::now();
        let err = output_bounded(
            Command::new("/bin/sh").args(["-c", "exec sleep 30"]),
            Duration::from_millis(300),
            1024,
        )
        .unwrap_err();
        assert!(err.contains("Timed out"), "{err}");
        assert!(t.elapsed() < Duration::from_secs(5));
    }
}
