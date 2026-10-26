//! `tashkeel`: CPU Arabic diacritization of text and DOCX with explicit model selection.

mod cli;
mod command;
mod document;
mod output;
mod setup;

use std::io::{self, IsTerminal, Write};
use std::ops::ControlFlow;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use anyhow::Context;
use tashkeel::{Diacritizer, InferenceOptions, Progress};

use crate::cli::{Args, Command, ModelCommand};
use crate::document::Document;
use crate::output::Output;

const EXIT_INTERRUPTED: u8 = 130;

fn main() -> ExitCode {
    let command = match command::parse() {
        Ok(command) => command,
        Err(code) => return code,
    };
    let result = match command {
        Command::Diacritize(args) => {
            Cancellation::install().and_then(|cancellation| diacritize(&args, &cancellation))
        }
        Command::Models(models) => match models.command {
            ModelCommand::List => command::list_models(),
            ModelCommand::Download(args) => Cancellation::install()
                .and_then(|cancellation| setup::download(&args, &cancellation)),
        },
        Command::Completions(args) => command::completions(&args),
        Command::Spec => command::spec(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error)
            if matches!(
                error.downcast_ref::<tashkeel::Error>(),
                Some(tashkeel::Error::Cancelled)
            ) =>
        {
            eprintln!("interrupted");
            ExitCode::from(EXIT_INTERRUPTED)
        }
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Shared Ctrl-C state, read during blocking setup, between chunks and before publication.
#[derive(Clone, Default)]
struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    fn install() -> anyhow::Result<Self> {
        let cancellation = Self::default();
        let flag = Arc::clone(&cancellation.0);
        ctrlc::set_handler(move || {
            if flag.swap(true, Ordering::Relaxed) {
                // A running native call or blocked output may never reach a checkpoint.
                // The second signal must work without taking locks or waiting for cleanup.
                std::process::exit(i32::from(EXIT_INTERRUPTED));
            }
        })
        .context("installing Ctrl-C handler")?;
        Ok(cancellation)
    }

    fn check(&self) -> tashkeel::Result<()> {
        if self.0.load(Ordering::Relaxed) {
            Err(tashkeel::Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Wait for blocking input/setup without making cancellation depend on its return.
    ///
    /// On cancellation the worker is detached until this CLI exits. Output destinations
    /// stay on the main thread and unwind normally; an interrupted model download may
    /// leave its private cache staging directory, never a partially published model.
    fn run_blocking<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        self.check()?;
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("tashkeel-setup".into())
            .spawn(move || {
                let _ = sender.send(operation());
            })
            .context("starting blocking input/setup worker")?;
        loop {
            self.check()?;
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(result) => {
                    self.check()?;
                    return result;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("blocking input/setup worker stopped unexpectedly");
                }
            }
        }
    }
}

fn diacritize(args: &Args, cancellation: &Cancellation) -> anyhow::Result<()> {
    let document = Document::read(args, cancellation);
    cancellation.check()?;
    let document = document?;
    let output = Output::prepare(args.output.as_deref(), args.force)?;
    setup::check_runtime_path(args.ort_library.as_deref())?;
    let directory = setup::model_directory(args, cancellation);
    cancellation.check()?;
    let directory = directory?;
    let options = InferenceOptions {
        threads: args.threads,
        chunk_size: args.chunk_size,
        runtime_path: args.ort_library.clone(),
    };
    let model = args.model;
    let mut diacritizer = cancellation.run_blocking(move || {
        Diacritizer::load(model, &directory, options).context("loading the selected model; install CPU ONNX Runtime and pass --ort-library PATH (see the README for setup instructions)")
    })?;
    let mut progress = ProgressOutput::new(args.quiet);
    let result = document.transform(|text| {
        cancellation.check()?;
        diacritizer.diacritize_with_progress(text, |state| {
            if cancellation.check().is_err() {
                return ControlFlow::Break(());
            }
            progress.update(state);
            ControlFlow::Continue(())
        })
    });
    drop(progress);
    cancellation.check()?;
    output.write(&result.context("diacritizing input")?, cancellation)
}

struct ProgressOutput {
    visible: bool,
    started: bool,
}

impl ProgressOutput {
    fn new(quiet: bool) -> Self {
        Self {
            visible: !quiet && io::stderr().is_terminal(),
            started: false,
        }
    }

    fn update(&mut self, progress: Progress) {
        if self.visible {
            self.started = true;
            eprint!("\rChunks: {}/{}   ", progress.completed, progress.total);
            let _ = io::stderr().flush();
        }
    }
}

impl Drop for ProgressOutput {
    fn drop(&mut self) {
        if self.started {
            eprintln!();
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    use super::Cancellation;

    struct Running(Child);

    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn wait_for_file(child: &mut Child, file: &Path) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !file.exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "signal helper exited early"
            );
            assert!(
                Instant::now() < deadline,
                "signal helper did not reach {}",
                file.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn interrupt(child: &Child) {
        assert!(
            Command::new("/bin/kill")
                .args(["-INT", &child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn second_ctrl_c_exits_even_when_work_never_returns() {
        const CHILD_DIRECTORY: &str = "TASHKEEL_TEST_BLOCKED_INTERRUPT_CHILD";
        if let Some(directory) = std::env::var_os(CHILD_DIRECTORY) {
            let directory = std::path::PathBuf::from(directory);
            let cancellation = Cancellation::install().unwrap();
            fs::write(directory.join("ready"), []).unwrap();
            while cancellation.check().is_ok() {
                std::thread::sleep(Duration::from_millis(10));
            }
            fs::write(directory.join("cancelled"), []).unwrap();
            // Simulate a native call that never returns to a cancellation checkpoint.
            loop {
                std::thread::park();
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let mut process = Running(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::second_ctrl_c_exits_even_when_work_never_returns",
                ])
                .env(CHILD_DIRECTORY, directory.path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        wait_for_file(&mut process.0, &directory.path().join("ready"));
        interrupt(&process.0);
        // Observe the first signal before sending another: Unix can coalesce signals.
        wait_for_file(&mut process.0, &directory.path().join("cancelled"));
        interrupt(&process.0);
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "second Ctrl-C did not force exit"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(status.code(), Some(130));
    }
}
