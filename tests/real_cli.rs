//! Opt-in end-to-end checks using verified local model files and CPU ONNX Runtime.
//!
//! Set `TASHKEEL_TEST_SHAKKELHA_DIR`, `TASHKEEL_TEST_CATT_DIR`, and (when needed)
//! `ORT_DYLIB_PATH`, then run `cargo test --test real_cli -- --ignored --test-threads=1`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn model_directory(model: &str) -> PathBuf {
    let key = match model {
        "shakkelha" => "TASHKEEL_TEST_SHAKKELHA_DIR",
        "catt" => "TASHKEEL_TEST_CATT_DIR",
        _ => unreachable!(),
    };
    std::env::var_os(key).map_or_else(
        || panic!("set {key} to the verified local model directory"),
        PathBuf::from,
    )
}

fn command(directory: &Path, model: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tashkeel"));
    command
        .current_dir(directory)
        .args(["--model", model, "--model-dir"])
        .arg(model_directory(model))
        .args(["--cache-dir", "unused-cache", "--quiet", "--threads", "1"])
        .stdin(Stdio::null());
    command
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn bare(text: &str) -> String {
    text.chars()
        .filter(|character| !('\u{064b}'..='\u{0652}').contains(character))
        .collect()
}

#[test]
#[ignore = "requires both verified local models and ONNX Runtime API 31"]
fn real_models_preserve_text_from_arguments_stdin_and_files() {
    for model in ["shakkelha", "catt"] {
        let directory = tempfile::tempdir().unwrap();
        let source =
            "\u{feff}  اللغة العربية جميلة، وفي عام 2026 نكتب Rust.\r\nهذا اختبار للتشكيل.\t ";
        let direct = command(directory.path(), model)
            .args(["--text", source])
            .output()
            .unwrap();
        success(&direct);
        let text = String::from_utf8(direct.stdout.clone()).unwrap();
        assert_eq!(bare(&text), bare(source));
        assert_ne!(text, bare(&text), "{model} must add diacritics");

        let mut child = command(directory.path(), model)
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let stdin = child.wait_with_output().unwrap();
        success(&stdin);
        assert_eq!(stdin.stdout, direct.stdout);

        let input_path = directory.path().join("book.txt");
        fs::write(&input_path, source).unwrap();
        let result = command(directory.path(), model)
            .args(["book.txt", "-o", "result.txt"])
            .output()
            .unwrap();
        success(&result);
        assert_eq!(result.stdout, [] as [u8; 0]);
        assert_eq!(fs::read(&input_path).unwrap(), source.as_bytes());
        assert_eq!(
            fs::read(directory.path().join("result.txt")).unwrap(),
            direct.stdout
        );

        let forced = command(directory.path(), model)
            .args(["book.txt", "-o", "book.txt", "--force"])
            .output()
            .unwrap();
        success(&forced);
        assert_eq!(forced.stdout, [] as [u8; 0]);
        assert_eq!(fs::read(&input_path).unwrap(), direct.stdout);
        assert!(!directory.path().join("unused-cache").exists());
    }
}

#[test]
#[ignore = "requires both verified local models and ONNX Runtime API 31"]
fn real_models_do_not_trim_empty_whitespace_or_append_a_newline() {
    for model in ["shakkelha", "catt"] {
        let directory = tempfile::tempdir().unwrap();
        for source in ["", "\t \r\n  ", "English 2026", "السلام عليكم"] {
            let result = command(directory.path(), model)
                .args(["--text", source])
                .output()
                .unwrap();
            success(&result);
            assert_eq!(bare(&String::from_utf8(result.stdout).unwrap()), source);
        }
        assert!(!directory.path().join("unused-cache").exists());
    }
}

fn document_text(bytes: &[u8]) -> Vec<String> {
    let mut paragraphs = Vec::new();
    tashkeel::docx::transform(bytes, |text| {
        paragraphs.push(text.to_owned());
        Ok(text.to_owned())
    })
    .unwrap();
    paragraphs
}

#[test]
#[ignore = "requires both verified local models and ONNX Runtime API 31"]
fn real_models_transform_docx_without_touching_input_or_stdout() {
    for model in ["shakkelha", "catt"] {
        let directory = tempfile::tempdir().unwrap();
        let source = include_bytes!("fixtures/formatted.docx");
        fs::write(directory.path().join("book.docx"), source).unwrap();
        let result = command(directory.path(), model)
            .args(["book.docx", "-o", "result.docx"])
            .output()
            .unwrap();
        success(&result);
        assert_eq!(result.stdout, [] as [u8; 0]);
        assert_eq!(
            fs::read(directory.path().join("book.docx")).unwrap(),
            source
        );
        let before = document_text(source);
        let after = document_text(&fs::read(directory.path().join("result.docx")).unwrap());
        assert_eq!(
            before.iter().map(|text| bare(text)).collect::<Vec<_>>(),
            after.iter().map(|text| bare(text)).collect::<Vec<_>>()
        );
        assert!(after.iter().any(|text| bare(text) != *text));
        assert!(!directory.path().join("unused-cache").exists());
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires verified Shakkelha files, ONNX Runtime API 31 and /bin/kill"]
fn ctrl_c_returns_130_without_publishing_partial_output() {
    use std::time::{Duration, Instant};

    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("book.txt"),
        "اللغة العربية جميلة. ".repeat(100_000),
    )
    .unwrap();
    fs::write(directory.path().join("result.txt"), "keep existing output").unwrap();
    let mut child = command(directory.path(), "shakkelha")
        .args([
            "book.txt",
            "-o",
            "result.txt",
            "--force",
            "--chunk-size",
            "100",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The private temporary output is created after the signal handler is installed and
    // before model setup. Its presence avoids racing a signal against process startup.
    let deadline = Instant::now() + Duration::from_secs(20);
    while fs::read_dir(directory.path()).unwrap().count() < 3 {
        if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "CLI did not prepare output: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("CLI did not honor Ctrl-C");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let result = child.wait_with_output().unwrap();
    assert_eq!(
        result.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(result.stdout, [] as [u8; 0]);
    assert_eq!(
        fs::read_to_string(directory.path().join("result.txt")).unwrap(),
        "keep existing output"
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}
