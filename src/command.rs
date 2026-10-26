//! Parsing, help and operations that need neither models nor a native runtime.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::process::ExitCode;

use usage_rs::ValidationError;
use usage_rs::embedded::{Exit, Outcome};
use usage_rs::install::{Env, Loading, OnForeign, Wrote};

use crate::cli::{Cli, Command, Completions, ModelCommand};
use crate::document;

/// Expand argument files and let Usage render help and validation errors.
pub(crate) fn parse() -> Result<Command, ExitCode> {
    let argv: Vec<OsString> = std::env::args_os().skip(1).collect();
    // Shell completion words are unfinished input; never expand their @files.
    if let Some(answer) = Cli::completion_request(&argv) {
        return Err(print_exit(&Exit {
            text: answer,
            stderr: false,
            code: 0,
        }));
    }
    let mut argv = usage_rs::response::expand(argv).map_err(|error| {
        print_exit(&Exit {
            text: format!("error: {error}\n"),
            stderr: true,
            code: 2,
        })
    })?;
    // Usage 7.1.2 treats a leading lone dash as a subcommand name. Route the
    // conventional stdin shorthand explicitly; completion input is untouched.
    if argv.first().is_some_and(|word| word == "-") {
        argv.insert(0, OsString::from("diacritize"));
    }
    let argv: Vec<_> = argv.iter().map(OsString::as_os_str).collect();
    if Cli::spec_request(&argv).is_some() {
        return Ok(Command::Spec);
    }
    match usage_rs::embedded::outcome(Cli::spec(), Cli::command(), &argv, parse_from) {
        Outcome::Parsed(command) => Ok(command),
        Outcome::Exit(exit) => Err(print_exit(&exit)),
    }
}

fn parse_from<'v>(argv: &[&'v OsStr]) -> Result<Command, usage_rs::Error<'static, 'v>> {
    let cli = Cli::parse_from(argv)?;
    if let Command::Diacritize(args) = &cli.command {
        validate(args).map_err(ValidationError::into_parse_error)?;
    }
    if let Command::Models(models) = &cli.command
        && let ModelCommand::Download(args) = &models.command
        && args
            .cache_dir
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Err(ValidationError::field("--cache-dir")
            .reason("must not be an empty path")
            .into_parse_error());
    }
    Ok(cli.command)
}

fn validate(args: &crate::cli::Args) -> Result<(), ValidationError> {
    for (flag, value) in [
        ("--threads", args.threads),
        ("--chunk-size", args.chunk_size),
    ] {
        if value == 0 {
            return Err(ValidationError::field(flag).reason("must be greater than zero"));
        }
    }
    if i32::try_from(args.threads).is_err() {
        return Err(ValidationError::field("--threads").reason("must be at most 2147483647"));
    }
    if args.chunk_size > usize::MAX - 2 || i64::try_from(args.chunk_size).is_err() {
        return Err(ValidationError::field("--chunk-size")
            .reason("must be a representable Unicode character count"));
    }
    for (flag, path) in [
        ("--model-dir", args.model_dir.as_ref()),
        ("--cache-dir", args.cache_dir.as_ref()),
        ("--ort-library", args.ort_library.as_ref()),
    ] {
        if path.is_some_and(|path| path.as_os_str().is_empty()) {
            return Err(ValidationError::field(flag).reason("must not be an empty path"));
        }
    }
    document::validate_paths(args)
}

fn print_exit(exit: &Exit) -> ExitCode {
    let result = if exit.stderr {
        io::stderr().lock().write_all(exit.text.as_bytes())
    } else {
        io::stdout().lock().write_all(exit.text.as_bytes())
    };
    match result {
        Ok(()) => ExitCode::from(u8::try_from(exit.code).unwrap_or(1)),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: writing CLI output: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Write a script to stdout or safely install it, optionally previewing its destination.
pub(crate) fn completions(args: &Completions) -> anyhow::Result<()> {
    let shell = usage_rs::complete::Shell::from_name(&args.shell)
        .expect("shell choices validated by Usage");
    if !args.install {
        io::stdout()
            .lock()
            .write_all(Cli::completion_script(shell).as_bytes())?;
        return Ok(());
    }
    let env = Env::from_process();
    let plan = if args.dry_run {
        let plan = Cli::completion_install_plan(shell, &env)?;
        println!("Would install completions to {}", plan.path.display());
        plan
    } else {
        let installed = Cli::install_completion(shell, &env, OnForeign::Refuse)?;
        let action = match installed.wrote {
            Wrote::Updated => "Updated",
            Wrote::Unchanged => "Already installed",
            _ => "Installed",
        };
        println!("{action}: {}", installed.plan.path.display());
        installed.plan
    };
    if let Loading::Manual { line, file, .. } = &plan.loading {
        println!("Add this to {file}:\n{line}");
    }
    if let Some(note) = plan.note {
        println!("Note: {note}");
    }
    Ok(())
}

/// Print the portable CLI definition.
pub(crate) fn spec() -> anyhow::Result<()> {
    io::stdout().lock().write_all(Cli::to_kdl().as_bytes())?;
    Ok(())
}

/// List backend IDs without touching the network, cache or native runtime.
pub(crate) fn list_models() -> anyhow::Result<()> {
    io::stdout().lock().write_all(
        b"shakkelha\tShakkelha character model\ncatt\tCATT encoder-only model\n\n\
Model validation: https://github.com/ieasybooks/tashkeel-rs/blob/main/docs/model-validation.md\n",
    )?;
    Ok(())
}
