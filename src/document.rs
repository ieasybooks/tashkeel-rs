//! Read and validate a single document without preparing a model.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use anyhow::{Context, ensure};
use usage_rs::ValidationError;

use crate::{Cancellation, cli::Args};

/// Input already decoded or checked before model setup.
pub(crate) enum Document {
    Text(String),
    Docx(Vec<u8>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Text,
    Docx,
}

fn extension(path: &Path) -> Option<String> {
    path.extension()?.to_str().map(str::to_ascii_lowercase)
}

fn input_format(path: &Path) -> Result<Format, ValidationError> {
    if path == Path::new("-") {
        return Ok(Format::Text);
    }
    match extension(path).as_deref() {
        Some("txt") => Ok(Format::Text),
        Some("docx") => Ok(Format::Docx),
        Some("doc") => Err(ValidationError::field("INPUT")
            .reason("legacy .doc files are not supported; convert the document to .docx first")),
        _ => Err(ValidationError::field("INPUT")
            .reason("expected one .txt or .docx file, or - for UTF-8 text from stdin")),
    }
}

/// Check format and destination arguments without filesystem or network access.
pub(crate) fn validate_paths(args: &Args) -> Result<(), ValidationError> {
    let format = match (&args.input, &args.text) {
        (Some(path), None) => input_format(path)?,
        (None, Some(_)) => Format::Text,
        _ => {
            return Err(
                ValidationError::field("INPUT").reason("supply exactly one INPUT or --text")
            );
        }
    };
    let output = args.output.as_deref();
    if output.is_some_and(|path| path.as_os_str().is_empty()) {
        return Err(ValidationError::field("--output").reason("must not be an empty path"));
    }
    if format == Format::Docx {
        if output.and_then(extension).as_deref() != Some("docx") {
            return Err(ValidationError::field("--output")
                .reason("DOCX input requires an explicit .docx output path (-o FILE.docx); binary DOCX cannot be written to stdout"));
        }
    } else if matches!(output.and_then(extension).as_deref(), Some("doc" | "docx")) {
        return Err(ValidationError::field("--output")
            .reason("text input produces UTF-8 text; use a .txt path or - for stdout"));
    }
    Ok(())
}

impl Document {
    /// Read UTF-8 without normalization and validate DOCX before downloads begin.
    pub(crate) fn read(args: &Args, cancellation: &Cancellation) -> anyhow::Result<Self> {
        if let Some(text) = &args.text {
            return Ok(Self::Text(text.clone()));
        }
        let path = args.input.clone().expect("INPUT validated by Usage");
        cancellation.run_blocking(move || Self::read_path(&path))
    }

    fn read_path(path: &Path) -> anyhow::Result<Self> {
        if path == Path::new("-") {
            return Self::read_text(io::stdin().lock()).context("reading UTF-8 text from stdin");
        }
        let metadata =
            fs::metadata(path).with_context(|| format!("reading input {}", path.display()))?;
        ensure!(
            metadata.is_file(),
            "input {} is not a regular file",
            path.display()
        );
        if extension(path).as_deref() == Some("docx") {
            let maximum = tashkeel::docx::Limits::default().max_archive_bytes;
            ensure!(
                metadata.len() <= u64::try_from(maximum)?,
                "DOCX input {} exceeds the archive size limit of {maximum} bytes",
                path.display()
            );
            let file = fs::File::open(path)
                .with_context(|| format!("opening DOCX input {}", path.display()))?;
            let bytes = read_archive(file, maximum)
                .with_context(|| format!("reading DOCX input {}", path.display()))?;
            // No separate validation API is required from the core. The identity transform
            // checks the whole archive; discard its output and retain the original bytes.
            tashkeel::docx::transform(&bytes, |text| Ok(text.to_owned()))
                .with_context(|| format!("validating DOCX input {}", path.display()))?;
            Ok(Self::Docx(bytes))
        } else {
            let bytes =
                fs::read(path).with_context(|| format!("reading input {}", path.display()))?;
            let text = String::from_utf8(bytes)
                .with_context(|| format!("input {} is not valid UTF-8 text", path.display()))?;
            Ok(Self::Text(text))
        }
    }

    fn read_text(mut reader: impl Read) -> anyhow::Result<Self> {
        let mut text = String::new();
        reader.read_to_string(&mut text)?;
        Ok(Self::Text(text))
    }

    /// Build the entire result in memory; a failed transform never reaches the destination.
    pub(crate) fn transform(
        &self,
        mut transform: impl FnMut(&str) -> tashkeel::Result<String>,
    ) -> tashkeel::Result<Vec<u8>> {
        match self {
            Self::Text(text) => transform(text).map(String::into_bytes),
            Self::Docx(bytes) => tashkeel::docx::transform(bytes, transform),
        }
    }
}

fn read_archive(reader: impl Read, maximum: usize) -> anyhow::Result<Vec<u8>> {
    // The extra byte detects files that grow after the metadata check. Never use
    // their advertised length as an unbounded allocation or read size.
    let limit = u64::try_from(maximum)?
        .checked_add(1)
        .context("DOCX archive limit is too large")?;
    let mut bytes = Vec::new();
    reader.take(limit).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= maximum,
        "DOCX input exceeds the archive size limit of {maximum} bytes"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{Document, read_archive};

    #[test]
    fn a_growing_archive_is_rejected_after_reading_only_the_limit_plus_one() {
        let mut input = std::io::Cursor::new([0_u8; 100]);
        let error = read_archive(&mut input, 16).unwrap_err();
        assert!(error.to_string().contains("archive size limit"));
        assert_eq!(input.position(), 17);
        assert_eq!(
            read_archive(&b"bounded input"[..], 13).unwrap(),
            b"bounded input"
        );
    }

    #[test]
    fn text_round_trip_preserves_bom_crlf_whitespace_and_final_newline_choice() {
        for source in ["", " \t\r\n", "\u{feff}  السلام\tعليكم\r\n\r\n", "مرحبا"] {
            let document = Document::read_text(source.as_bytes()).unwrap();
            let result = document.transform(|text| Ok(text.to_owned())).unwrap();
            assert_eq!(result, source.as_bytes());
        }
    }

    #[test]
    fn stdin_rejects_invalid_utf8_instead_of_replacing_bytes() {
        assert!(Document::read_text(&[0xff, 0xfe][..]).is_err());
    }

    #[test]
    fn a_failed_transform_has_no_result_to_publish() {
        let document = Document::Text("السلام عليكم".into());
        let result = document.transform(|_| Err(tashkeel::Error::Cancelled));
        assert!(matches!(result, Err(tashkeel::Error::Cancelled)));
    }
}
