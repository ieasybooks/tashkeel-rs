//! Synchronous CPU inference using dynamically loaded ONNX Runtime.

use std::{
    fmt,
    ops::ControlFlow,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::{
    Error, Result,
    text::{self, tokenizer::Tokenizer},
};

mod runtime;

/// Supported model architectures and their embedded vocabularies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Model {
    /// Shakkelha normalized, big-data, average-of-20 bidirectional LSTM model.
    Shakkelha,
    /// CATT encoder-only transformer and its classification head.
    Catt,
}

impl Model {
    /// Stable model identifier used in cache directories and configuration.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Shakkelha => "shakkelha",
            Self::Catt => "catt",
        }
    }
}

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Model {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "shakkelha" => Ok(Self::Shakkelha),
            "catt" => Ok(Self::Catt),
            _ => Err(Error::InvalidInput(format!(
                "unknown model {value:?}; expected shakkelha or catt"
            ))),
        }
    }
}

/// Resource and text-chunking options for one diacritizer.
#[derive(Clone, Debug)]
pub struct InferenceOptions {
    /// CPU threads within a graph operation. Must be positive; default is one.
    /// Each session uses its own pool, including with a host environment.
    pub threads: usize,
    /// Maximum source Unicode scalars per chunk, including existing marks.
    /// Must be positive. CATT also limits each chunk to 1,024 normalized tokens.
    pub chunk_size: usize,
    /// ONNX Runtime shared library path. Otherwise use `ORT_DYLIB_PATH`, then
    /// a library beside the executable. Windows and macOS require one of these;
    /// other platforms can fall back to the platform loader's library name.
    /// Requires API 31 (1.31+). Must be `None` with `load_with_environment`.
    /// ONNX Runtime is process-global; select its path before the first load.
    pub runtime_path: Option<PathBuf>,
}

impl Default for InferenceOptions {
    fn default() -> Self {
        Self {
            threads: 1,
            chunk_size: 500,
            runtime_path: None,
        }
    }
}

impl InferenceOptions {
    fn validate(&self) -> Result<()> {
        if self.threads == 0 || i32::try_from(self.threads).is_err() {
            return Err(Error::InvalidInput(
                "threads must be between 1 and 2147483647".into(),
            ));
        }
        if self.chunk_size == 0
            || self.chunk_size > usize::MAX - 2
            || i64::try_from(self.chunk_size).is_err()
        {
            return Err(Error::InvalidInput(
                "chunk_size must be a positive, representable Unicode scalar count".into(),
            ));
        }
        if self
            .runtime_path
            .as_ref()
            .is_some_and(|p| p.as_os_str().is_empty())
        {
            return Err(Error::InvalidInput("runtime_path must not be empty".into()));
        }
        Ok(())
    }
}

/// Chunk progress, including chunks that require no model inference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Progress {
    /// Number of completely processed chunks.
    pub completed: usize,
    /// Total chunks for this call.
    pub total: usize,
}

/// A reusable, synchronous CPU model session.
///
/// Each call runs exact-length inputs with batch size one. Original base
/// characters and document structure are preserved; only U+064B–U+0652 marks
/// are replaced. CATT normalizes input positions and maps predictions back to
/// source positions. Multi-letter presentation ligatures remain unmarked
/// because their internal marks cannot be represented without changing text.
pub struct Diacritizer {
    backend: runtime::Backend,
    tokenizer: Tokenizer,
    options: InferenceOptions,
}

impl Diacritizer {
    /// Load local model files; this method never downloads artifacts.
    ///
    /// Shakkelha requires `model.onnx`; CATT requires `encoder.onnx` and
    /// `decoder.onnx`. The caller is responsible for verifying artifact hashes
    /// (the [`crate::models`] helpers do this). Shapes and types are checked here.
    /// If the application already initializes `ort`, use
    /// [`Self::load_with_environment`] instead: `ort` permits only one environment.
    pub fn load(model: Model, model_dir: &Path, options: InferenceOptions) -> Result<Self> {
        Self::load_inner(model, model_dir, options, None)
    }

    /// Load local model files using the application's existing `ort` environment.
    ///
    /// The environment must come from the same pinned `ort` crate (2.0.0-rc.15,
    /// API 31). Its native runtime selection and trust are the host's responsibility.
    /// `options.runtime_path` must be `None`; this method performs no runtime path
    /// discovery and does not read `ORT_DYLIB_PATH` or initialize an environment.
    ///
    /// Sessions retain shared ownership of the environment, so the caller may
    /// drop its handle after loading. Keep a handle to create later engines;
    /// `ort` cannot create a replacement environment even after all handles drop.
    /// Sessions use only CPU execution, ignore the environment's default execution
    /// providers, and use independent thread pools honoring `options.threads`.
    /// Other host settings, including logging and telemetry, remain in effect.
    /// Artifact verification is the caller's responsibility, as with [`Self::load`].
    pub fn load_with_environment(
        model: Model,
        model_dir: &Path,
        options: InferenceOptions,
        environment: &ort::environment::Environment,
    ) -> Result<Self> {
        if options.runtime_path.is_some() {
            return Err(Error::InvalidInput(
                "runtime_path must be None when supplying a host ONNX Runtime environment".into(),
            ));
        }
        Self::load_inner(model, model_dir, options, Some(environment))
    }

    fn load_inner(
        model: Model,
        model_dir: &Path,
        options: InferenceOptions,
        environment: Option<&ort::environment::Environment>,
    ) -> Result<Self> {
        options.validate()?;
        let tokenizer = Tokenizer::new(model)?;
        let backend = runtime::Backend::load(model, model_dir, &options, environment)?;
        Ok(Self {
            backend,
            tokenizer,
            options,
        })
    }

    /// Replace common tashkeel while preserving the original source text.
    /// There is no whole-document character limit.
    pub fn diacritize(&mut self, text: &str) -> Result<String> {
        self.diacritize_with_progress(text, |_| ControlFlow::Continue(()))
    }

    /// Diacritize with progress before the first chunk and after each chunk.
    ///
    /// Returning `ControlFlow::Break(())` cancels with [`Error::Cancelled`]; no
    /// partial output is returned. A running native inference call finishes
    /// before cancellation is observed. Empty input reports `(0, 0)` once.
    pub fn diacritize_with_progress(
        &mut self,
        text: &str,
        mut progress: impl FnMut(Progress) -> ControlFlow<()>,
    ) -> Result<String> {
        let chunks = text::chunks(text, self.options.chunk_size, &self.tokenizer);
        let total = chunks.len();
        if progress(Progress {
            completed: 0,
            total,
        })
        .is_break()
        {
            return Err(Error::Cancelled);
        }
        let mut output = String::with_capacity(text.len());
        for (index, chunk) in chunks.into_iter().enumerate() {
            let encoded = self.tokenizer.encode(chunk);
            if encoded.has_targets() {
                let labels = self
                    .backend
                    .predict(&encoded.ids, self.tokenizer.class_count())?;
                output.push_str(&self.tokenizer.render(chunk, &encoded, &labels)?);
            } else {
                output.extend(chunk.chars().filter(|&c| !text::is_tashkeel(c)));
            }
            if progress(Progress {
                completed: index + 1,
                total,
            })
            .is_break()
            {
                return Err(Error::Cancelled);
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests;
