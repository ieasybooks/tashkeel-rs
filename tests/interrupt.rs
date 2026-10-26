//! Signal regressions exercise blocked input and a stalled local download proxy.
#![cfg(unix)]

use std::fs;
use std::io;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Ensure a failed assertion cannot leave a blocked child behind.
struct Running(Option<Child>);

impl Running {
    fn child(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }

    fn signal(&mut self) {
        assert!(
            Command::new("/bin/kill")
                .args(["-INT", &self.child().id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }

    fn wait_until(&mut self, description: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready() {
            assert!(
                self.child().try_wait().unwrap().is_none(),
                "child exited before {description}"
            );
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {description}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn finish(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child().try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "Ctrl-C did not stop the blocked CLI"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        self.0.take().unwrap().wait_with_output().unwrap()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tashkeel"));
    command
        .current_dir(directory)
        .env("NO_COLOR", "1")
        .env("ORT_DYLIB_PATH", directory.join("no-runtime"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn interrupted(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("interrupted"));
}

// Linux exposes signal-handler readiness without introducing test-only CLI flags,
// arbitrary startup sleeps or closing the input pipe as part of the assertion.
#[cfg(target_os = "linux")]
#[test]
fn first_ctrl_c_exits_while_stdin_pipe_stays_open() {
    for destination in [None, Some(false), Some(true)] {
        let directory = tempfile::tempdir().unwrap();
        let output_path = directory.path().join("result.txt");
        let mut command = command(directory.path());
        command
            .args([
                "-",
                "--model",
                "shakkelha",
                "--offline",
                "--cache-dir",
                "cache",
                "--quiet",
            ])
            .stdin(Stdio::piped());
        if let Some(force) = destination {
            command.args(["-o", "result.txt"]);
            if force {
                fs::write(&output_path, "keep existing output").unwrap();
                command.arg("--force");
            }
        }
        let mut process = Running(Some(command.spawn().unwrap()));
        // Keep this handle outside Child so wait_with_output cannot close it and
        // accidentally make a blocked read finish, hiding the original regression.
        let input = process.child().stdin.take().unwrap();
        let status = format!("/proc/{}/status", process.child().id());
        process.wait_until("SIGINT handler installation", || {
            fs::read_to_string(&status)
                .unwrap_or_default()
                .lines()
                .find_map(|line| line.strip_prefix("SigCgt:"))
                .and_then(|mask| u64::from_str_radix(mask.trim(), 16).ok())
                .is_some_and(|mask| mask & 2 != 0)
        });
        process.signal();
        interrupted(&process.finish());
        if destination == Some(true) {
            assert_eq!(
                fs::read_to_string(&output_path).unwrap(),
                "keep existing output"
            );
        } else {
            assert!(!output_path.exists());
        }
        assert!(!directory.path().join("cache").exists());
        drop(input);
    }
}

#[test]
fn first_ctrl_c_exits_during_model_setup_and_explicit_download() {
    for download_only in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("cache");
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let url = format!("http://{}", proxy.local_addr().unwrap());
        let mut command = command(directory.path());
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env(name, &url);
        }
        command.env_remove("NO_PROXY").env_remove("no_proxy");
        if download_only {
            command.args(["models", "download"]);
        } else {
            fs::write(directory.path().join("result.txt"), "keep existing output").unwrap();
            command.args(["--text", "السلام", "-o", "result.txt", "--force"]);
        }
        command.args(["--model", "shakkelha", "--cache-dir", "cache", "--quiet"]);
        let mut process = Running(Some(command.spawn().unwrap()));
        let mut connection = None;
        process.wait_until("download proxy connection", || match proxy.accept() {
            Ok((stream, _)) => {
                connection = Some(stream);
                true
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => false,
            Err(error) => panic!("accepting download proxy connection: {error}"),
        });
        // Keep the connection open without answering CONNECT. Only Ctrl-C, not
        // EOF or a network timeout, can unblock the CLI within the test deadline.
        process.signal();
        interrupted(&process.finish());
        assert!(!tashkeel::models::model_dir(tashkeel::Model::Shakkelha, &cache).exists());
        if !download_only {
            assert_eq!(
                fs::read_to_string(directory.path().join("result.txt")).unwrap(),
                "keep existing output"
            );
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
        }
        drop(connection);
    }
}
