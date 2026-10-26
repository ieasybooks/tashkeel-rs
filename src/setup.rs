//! Local model verification, explicit downloads and runtime-path checks.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use tashkeel::models;

use crate::Cancellation;
use crate::cli::{Args, Download};

fn cache_directory(explicit: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_owned());
    }
    dirs::cache_dir()
        .map(|path| path.join("tashkeel"))
        .context("cannot locate the user cache directory; pass --cache-dir DIR")
}

/// Local model directories and offline caches are always verified without network access.
pub(crate) fn model_directory(args: &Args, cancellation: &Cancellation) -> anyhow::Result<PathBuf> {
    let model = args.model;
    if let Some(directory) = &args.model_dir {
        let directory = directory.clone();
        return cancellation.run_blocking(move || {
            models::verify(model, &directory).with_context(|| {
                format!("verifying --model-dir {} for {model}; supply the pinned files for this model (no downloads are attempted with --model-dir)", directory.display())
            })?;
            Ok(directory)
        });
    }
    let cache = cache_directory(args.cache_dir.as_deref())?;
    if args.offline {
        let directory = models::model_dir(model, &cache);
        return cancellation.run_blocking(move || {
            models::verify(model, &directory).with_context(|| {
                format!("offline: {model} model files are missing or invalid in {}; run `tashkeel models download --model {model}` with this --cache-dir while online, or pass --model-dir DIR", directory.display())
            })?;
            Ok(directory)
        });
    }
    if !args.quiet {
        eprintln!(
            "Preparing verified {} model files in {}",
            args.model,
            cache.display()
        );
    }
    cancellation.run_blocking(move || {
        models::ensure(model, &cache).with_context(|| {
            format!("preparing {model} model; check network access or supply verified files with --model-dir DIR")
        })
    })
}

/// Reject a missing explicit library before any model download.
pub(crate) fn check_runtime_path(path: Option<&Path>) -> anyhow::Result<()> {
    if let Some(path) = path {
        let metadata = fs::metadata(path).with_context(|| {
            format!("reading ONNX Runtime library {}; install CPU ONNX Runtime and pass --ort-library PATH (see the README for setup instructions)", path.display())
        })?;
        ensure!(
            metadata.is_file(),
            "ONNX Runtime library {} is not a regular file; pass --ort-library PATH to the shared library",
            path.display()
        );
    }
    Ok(())
}

/// Download weights without initializing the native runtime, then print their directory.
pub(crate) fn download(args: &Download, cancellation: &Cancellation) -> anyhow::Result<()> {
    let cache = cache_directory(args.cache_dir.as_deref())?;
    if !args.quiet {
        eprintln!(
            "Preparing verified {} model files in {}",
            args.model,
            cache.display()
        );
    }
    let model = args.model;
    let directory = cancellation.run_blocking(move || {
        models::ensure(model, &cache).with_context(|| format!("downloading and verifying {model}"))
    })?;
    writeln!(io::stdout().lock(), "{}", directory.display())?;
    Ok(())
}
