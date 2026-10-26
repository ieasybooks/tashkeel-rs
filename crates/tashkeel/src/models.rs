//! Pinned model artifacts, verified downloads, and an atomic on-disk cache.
//!
//! These functions are blocking. Applications with an async executor should call
//! them on a blocking worker, as they do for inference.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::Client;
use sha2::{Digest, Sha256};

use crate::{Error, Model, Result};

/// A file belonging to a pinned model revision.
#[derive(Debug, Clone, Copy)]
pub struct ModelFile {
    /// File name within the model directory.
    pub name: &'static str,
    /// Expected uncompressed size, in bytes.
    pub bytes: u64,
    /// SHA-256 digest of the complete file.
    pub sha256: &'static str,
}

const SHAKKELHA_FILES: &[ModelFile] = &[ModelFile {
    name: "model.onnx",
    bytes: 10_777_454,
    sha256: "41ed03075e80847aaef2bfdb65f123b7f225ff6a179a507b1ec3e6152184fa71",
}];

const CATT_FILES: &[ModelFile] = &[
    ModelFile {
        name: "encoder.onnx",
        bytes: 77_915_147,
        sha256: "437a99c032ec7de4249280efbc4dce2d0532f95c9fe8356debf24ce05a34ba6a",
    },
    ModelFile {
        name: "decoder.onnx",
        bytes: 37_294,
        sha256: "e6575bdba8623bf51a6d459fc4955e8a98aa3c6c2ffc5abffac26e883792bc80",
    },
];

const SHAKKELHA_URL: &str = "https://github.com/ieasybooks/tashkeel-rs/releases/download/models-v1/shakkelha_normalized_big_avg20_sigmoid.onnx";
const CATT_URL: &str = "https://github.com/abjadai/catt/releases/download/v2/eo_model_onnx.zip";
const HEX: &[u8; 16] = b"0123456789abcdef";
const CATT_ARCHIVE: ModelFile = ModelFile {
    name: "eo_model_onnx.zip",
    bytes: 72_109_998,
    sha256: "5343bfdfda70c981b587811b79348b8dde91f65c0a551ca98f232ea5cb62ceac",
};

/// The immutable model files required by a backend.
pub fn files(model: Model) -> &'static [ModelFile] {
    match model {
        Model::Shakkelha => SHAKKELHA_FILES,
        Model::Catt => CATT_FILES,
    }
}

/// Location of this pinned revision beneath a caller-selected cache directory.
pub fn model_dir(model: Model, cache_root: &Path) -> PathBuf {
    cache_root.join(model.as_str()).join("v1")
}

/// Verify both size and SHA-256 of every model file, including cached files.
///
/// Local model directories use the same pinned contract as downloaded models.
pub fn verify(model: Model, directory: &Path) -> Result<()> {
    for expected in files(model) {
        verify_file(&directory.join(expected.name), expected)?;
    }
    Ok(())
}

/// Return a verified cached model, downloading it if absent.
///
/// A corrupt existing cache is reported without removing the user's files. New
/// models are downloaded to a private temporary directory and published only
/// after every digest passes. Concurrent downloads cannot expose partial models.
pub fn ensure(model: Model, cache_root: &Path) -> Result<PathBuf> {
    let directory = model_dir(model, cache_root);
    if directory.exists() {
        verify(model, &directory)?;
        return Ok(directory);
    }
    let parent = directory
        .parent()
        .ok_or_else(|| Error::Model("invalid cache path".into()))?;
    fs::create_dir_all(parent)?;
    let staging = tempfile::Builder::new()
        .prefix(".download-")
        .tempdir_in(parent)?;
    let client = Client::builder()
        .user_agent(concat!("tashkeel/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_mins(15))
        .build()
        .map_err(|error| Error::Model(error.to_string()))?;
    match model {
        Model::Shakkelha => download(
            &client,
            SHAKKELHA_URL,
            &SHAKKELHA_FILES[0],
            &staging.path().join("model.onnx"),
        )?,
        Model::Catt => {
            let archive = staging.path().join(CATT_ARCHIVE.name);
            download(&client, CATT_URL, &CATT_ARCHIVE, &archive)?;
            extract_catt(&archive, staging.path())?;
            fs::remove_file(archive)?;
        }
    }
    verify(model, staging.path())?;
    match fs::rename(staging.path(), &directory) {
        Ok(()) => Ok(directory),
        Err(_) if directory.exists() => {
            // Another process may have published the same immutable revision.
            verify(model, &directory)?;
            Ok(directory)
        }
        Err(error) => Err(error.into()),
    }
}

fn verify_file(path: &Path, expected: &ModelFile) -> Result<()> {
    let mut file = File::open(path)
        .map_err(|error| Error::Model(format!("cannot open {}: {error}", path.display())))?;
    if !file.metadata()?.is_file() {
        return Err(Error::Model(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    if file.metadata()?.len() != expected.bytes {
        return Err(Error::Model(format!(
            "{} has an unexpected size; expected {} bytes. Use a fresh model directory",
            path.display(),
            expected.bytes
        )));
    }
    check_stream(&mut file, &mut std::io::sink(), expected)
        .map_err(|error| Error::Model(format!("{}: {error}", path.display())))
}

fn check_stream(
    reader: &mut impl Read,
    writer: &mut impl Write,
    expected: &ModelFile,
) -> Result<()> {
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .ok_or_else(|| Error::Model("download size overflow".into()))?;
        if bytes > expected.bytes {
            return Err(Error::Model(format!(
                "{} exceeds its pinned size",
                expected.name
            )));
        }
        digest.update(&buffer[..count]);
        writer.write_all(&buffer[..count])?;
    }
    if bytes != expected.bytes {
        return Err(Error::Model(format!(
            "{} is incomplete: expected {} bytes, received {bytes}",
            expected.name, expected.bytes
        )));
    }
    let mut actual = String::with_capacity(64);
    for byte in digest.finalize() {
        actual.push(char::from(HEX[usize::from(byte >> 4)]));
        actual.push(char::from(HEX[usize::from(byte & 15)]));
    }
    if actual != expected.sha256 {
        return Err(Error::Model(format!("{} SHA-256 mismatch", expected.name)));
    }
    Ok(())
}

fn download(client: &Client, url: &str, expected: &ModelFile, path: &Path) -> Result<()> {
    let mut response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| Error::Model(format!("download {url}: {error}")))?;
    if let Some(length) = response.content_length()
        && length != expected.bytes
    {
        return Err(Error::Model(format!(
            "{} has unexpected Content-Length {length}",
            expected.name
        )));
    }
    let mut file = File::create(path)?;
    check_stream(&mut response, &mut file, expected)?;
    file.sync_all()?;
    Ok(())
}

fn extract_catt(archive: &Path, directory: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)
        .map_err(|error| Error::Model(format!("invalid model archive: {error}")))?;
    if zip.len() != CATT_FILES.len() {
        return Err(Error::Model(
            "unexpected entries in the CATT archive".into(),
        ));
    }
    for expected in CATT_FILES {
        // Extract only known names, never paths supplied by the archive.
        let mut entry = zip
            .by_name(expected.name)
            .map_err(|error| Error::Model(format!("missing {}: {error}", expected.name)))?;
        if entry.size() != expected.bytes {
            return Err(Error::Model(format!(
                "unexpected size of {} in archive",
                expected.name
            )));
        }
        let mut out = File::create(directory.join(expected.name))?;
        check_stream(&mut entry, &mut out, expected)?;
        out.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    const HELLO: ModelFile = ModelFile {
        name: "test.onnx",
        bytes: 5,
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    };

    #[test]
    fn validates_payload_and_rejects_corruption_and_truncation() {
        let mut output = Vec::new();
        check_stream(&mut &b"hello"[..], &mut output, &HELLO).unwrap();
        assert_eq!(output, b"hello");
        for input in [&b"hell"[..], &b"hellO"[..], &b"hello!"[..]] {
            assert!(check_stream(&mut &input[..], &mut Vec::new(), &HELLO).is_err());
        }
    }

    #[test]
    fn detects_same_length_corrupt_cache_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.onnx");
        fs::write(&file, b"hello").unwrap();
        verify_file(&file, &HELLO).unwrap();
        fs::write(&file, b"hellO").unwrap();
        assert!(
            verify_file(&file, &HELLO)
                .unwrap_err()
                .to_string()
                .contains("SHA-256")
        );
    }

    #[test]
    fn downloads_and_checks_an_http_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.ends_with(b"\r\n\r\n") {
                let amount = stream.read(&mut buffer).unwrap();
                assert!(amount > 0, "client closed before sending its headers");
                request.extend_from_slice(&buffer[..amount]);
                assert!(request.len() <= 4096, "unexpectedly large test request");
            }
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                )
                .unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("model");
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        download(&client, &format!("http://{address}/model"), &HELLO, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"hello");
        server.join().unwrap();
    }

    #[test]
    fn an_invalid_existing_cache_is_never_deleted() {
        let cache = tempfile::tempdir().unwrap();
        let dir = model_dir(Model::Shakkelha, cache.path());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("model.onnx"), b"user file").unwrap();
        assert!(ensure(Model::Shakkelha, cache.path()).is_err());
        assert_eq!(fs::read(dir.join("model.onnx")).unwrap(), b"user file");
    }
}
