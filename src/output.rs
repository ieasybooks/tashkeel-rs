//! Atomic file publication and exact text output.

use std::fs::{self, Permissions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail, ensure};
use tempfile::NamedTempFile;

use crate::Cancellation;

/// A destination prepared before model setup, without truncating any existing file.
pub(crate) enum Output {
    Stdout,
    File {
        path: PathBuf,
        temporary: NamedTempFile,
        force: bool,
        permissions: Option<Permissions>,
    },
}

impl Output {
    /// Check collisions and create a temporary file in the destination's directory.
    pub(crate) fn prepare(path: Option<&Path>, force: bool) -> anyhow::Result<Self> {
        let Some(path) = path.filter(|path| *path != Path::new("-")) else {
            return Ok(Self::Stdout);
        };
        let permissions = match fs::symlink_metadata(path) {
            Ok(metadata) => {
                ensure!(
                    force,
                    "output {} already exists; use --force to replace it",
                    path.display()
                );
                ensure!(
                    metadata.is_file(),
                    "output {} is not a regular file; refusing to replace it",
                    path.display()
                );
                Some(metadata.permissions())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("checking output {}", path.display()));
            }
        };
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let temporary = NamedTempFile::new_in(parent).with_context(|| {
            format!(
                "preparing output {} (its parent directory must exist and be writable)",
                path.display()
            )
        })?;
        Ok(Self::File {
            path: path.to_owned(),
            temporary,
            force,
            permissions,
        })
    }

    /// Publish only a complete result, checking cancellation immediately before publication.
    pub(crate) fn write(self, bytes: &[u8], cancellation: &Cancellation) -> anyhow::Result<()> {
        cancellation.check()?;
        match self {
            Self::Stdout => {
                let mut stdout = io::stdout().lock();
                stdout
                    .write_all(bytes)
                    .context("writing result to stdout")?;
                stdout.flush().context("flushing stdout")?;
            }
            Self::File {
                path,
                mut temporary,
                force,
                permissions,
            } => {
                temporary
                    .write_all(bytes)
                    .with_context(|| format!("writing temporary output for {}", path.display()))?;
                if let Some(permissions) = permissions {
                    temporary.as_file().set_permissions(permissions)?;
                }
                temporary
                    .as_file()
                    .sync_all()
                    .with_context(|| format!("flushing temporary output for {}", path.display()))?;
                cancellation.check()?;
                let result = if force {
                    temporary.persist(&path)
                } else {
                    temporary.persist_noclobber(&path)
                };
                if let Err(error) = result {
                    if !force && error.error.kind() == io::ErrorKind::AlreadyExists {
                        bail!(
                            "output {} already exists; use --force to replace it",
                            path.display()
                        );
                    }
                    return Err(error.error)
                        .with_context(|| format!("publishing output {}", path.display()));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::Ordering;

    use super::Output;
    use crate::Cancellation;

    #[test]
    fn a_destination_created_during_inference_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.txt");
        let output = Output::prepare(Some(&path), false).unwrap();
        fs::write(&path, "written by someone else").unwrap();
        let error = output
            .write(b"result", &Cancellation::default())
            .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "written by someone else"
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn only_force_replaces_a_file_and_publishes_exact_bytes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.txt");
        fs::write(&path, "original").unwrap();
        assert!(Output::prepare(Some(&path), false).is_err());
        let output = Output::prepare(Some(&path), true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        let result = "\u{feff}\tالسَّلَامُ\r\n  ";
        output
            .write(result.as_bytes(), &Cancellation::default())
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), result.as_bytes());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn cancellation_and_dropped_results_leave_original_files_intact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.txt");
        fs::write(&path, "original").unwrap();
        let cancellation = Cancellation::default();
        cancellation.0.store(true, Ordering::Relaxed);
        let output = Output::prepare(Some(&path), true).unwrap();
        assert!(output.write(b"unfinished", &cancellation).is_err());
        drop(Output::prepare(Some(&path), true).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn cancellation_does_not_create_the_requested_output() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.txt");
        let cancellation = Cancellation::default();
        let output = Output::prepare(Some(&path), false).unwrap();
        cancellation.0.store(true, Ordering::Relaxed);
        assert!(output.write(b"unfinished", &cancellation).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn even_dangling_symlinks_are_protected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.txt");
        std::os::unix::fs::symlink(directory.path().join("missing.txt"), &path).unwrap();
        assert!(Output::prepare(Some(&path), false).is_err());
        assert!(Output::prepare(Some(&path), true).is_err());
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
