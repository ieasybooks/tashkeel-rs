//! Model-free CLI behavior: useful helpers, early validation and safe output handling.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tashkeel"));
    command
        .current_dir(directory)
        .env("HOME", directory)
        .env("USERPROFILE", directory)
        .env("XDG_CACHE_HOME", directory.join("cache"))
        .env("XDG_CONFIG_HOME", directory.join("config"))
        .env("LOCALAPPDATA", directory.join("AppData/Local"))
        .env("APPDATA", directory.join("AppData/Roaming"))
        .env(
            "ORT_DYLIB_PATH",
            directory.join("missing-onnxruntime-library"),
        )
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("CLICOLOR_FORCE")
        .env("NO_COLOR", "1")
        .env("PATH", "")
        .stdin(Stdio::null());
    command
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn success(output: &Output) {
    assert!(output.status.success(), "{}", stderr(output));
    assert!(output.stderr.is_empty(), "{}", stderr(output));
}

fn failure(output: &Output, code: i32, message: &str) {
    assert_eq!(output.status.code(), Some(code), "{}", stderr(output));
    assert!(output.stdout.is_empty(), "{}", stdout(output));
    assert!(
        stderr(output).contains(message),
        "expected {message:?}: {}",
        stderr(output)
    );
}

#[test]
fn help_version_spec_and_model_list_need_no_model_or_native_runtime() {
    let directory = tempfile::tempdir().unwrap();
    for args in [
        vec!["--help"],
        vec!["diacritize", "--help"],
        vec!["models", "--help"],
        vec!["models", "download", "--help"],
        vec!["--version"],
        vec!["spec"],
        vec!["__usage_spec__"],
        vec!["models", "list"],
    ] {
        let output = command(directory.path()).args(&args).output().unwrap();
        success(&output);
        assert!(!output.stdout.is_empty(), "{args:?}");
    }
    let help = stdout(&command(directory.path()).arg("--help").output().unwrap());
    for text in [
        "diacritize",
        "completions",
        "models",
        "spec",
        "--model",
        "@FILE",
        "--force",
    ] {
        assert!(help.contains(text), "missing {text}: {help}");
    }
    let spec = stdout(&command(directory.path()).arg("spec").output().unwrap());
    assert_eq!(
        spec,
        stdout(
            &command(directory.path())
                .arg("__usage_spec__")
                .output()
                .unwrap()
        )
    );
    assert!(spec.contains("shakkelha"));
    assert!(spec.contains("catt"));
    let models = stdout(
        &command(directory.path())
            .args(["models", "list"])
            .output()
            .unwrap(),
    );
    assert!(models.contains("shakkelha"));
    assert!(models.contains("catt"));
    assert!(models.contains("Model validation:"));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn input_and_explicit_model_are_required() {
    let directory = tempfile::tempdir().unwrap();
    for args in [
        vec![],
        vec!["diacritize"],
        vec!["--text", "السلام"],
        vec!["input.txt"],
        vec!["--model", "shakkelha"],
        vec!["--text", "السلام", "input.txt", "--model", "shakkelha"],
        vec!["first.txt", "second.txt", "--model", "shakkelha"],
        vec!["models", "download"],
    ] {
        let output = command(directory.path()).args(&args).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            stderr(&output)
        );
        assert!(output.stdout.is_empty(), "{args:?}: {}", stdout(&output));
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn invalid_options_fail_before_model_setup() {
    let directory = tempfile::tempdir().unwrap();
    for args in [
        vec!["--threads", "0"],
        vec!["--threads", "-1"],
        vec!["--threads", "2147483648"],
        vec!["--threads", "many"],
        vec!["--chunk-size", "0"],
        vec!["--chunk-size", "18446744073709551615"],
        vec!["--chunk-size", "NaN"],
        vec!["--model-dir", ""],
        vec!["--cache-dir", ""],
        vec!["--ort-library", ""],
        vec!["--output", ""],
        vec!["--typo"],
    ] {
        let output = command(directory.path())
            .args(["--text", "السلام", "--model", "shakkelha"])
            .args(&args)
            .output()
            .unwrap();
        failure(&output, 2, "error:");
    }
    for args in [
        vec!["--text", "السلام", "--model", "automatic"],
        vec!["models", "download", "--model", "automatic"],
        vec!["models", "download", "--model", "catt", "--cache-dir", ""],
        vec!["models", "list", "--typo"],
        vec!["models", "download", "--model", "catt", "--offline"],
    ] {
        failure(
            &command(directory.path()).args(args).output().unwrap(),
            2,
            "error:",
        );
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn unsupported_formats_and_binary_stdout_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    for (args, message) in [
        (vec!["legacy.doc"], "convert the document to .docx"),
        (vec!["archive.pdf"], ".txt or .docx"),
        (vec!["document.docx"], "explicit .docx output path"),
        (
            vec!["document.docx", "-o", "-"],
            "explicit .docx output path",
        ),
        (
            vec!["document.docx", "-o", "out.txt"],
            "explicit .docx output path",
        ),
        (
            vec!["--text", "السلام", "-o", "out.docx"],
            "text input produces UTF-8 text",
        ),
    ] {
        let output = command(directory.path())
            .args(args)
            .args(["--model", "shakkelha"])
            .output()
            .unwrap();
        failure(&output, 2, message);
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn invalid_files_fail_before_a_cache_or_output_is_created() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("invalid.txt"), [0xff, 0xfe]).unwrap();
    fs::write(directory.path().join("broken.docx"), b"not a ZIP archive").unwrap();
    fs::create_dir(directory.path().join("directory.txt")).unwrap();
    for (input, output, message) in [
        ("missing.txt", "out.txt", "reading input missing.txt"),
        ("invalid.txt", "out.txt", "not valid UTF-8"),
        ("broken.docx", "out.docx", "validating DOCX input"),
        ("directory.txt", "out.txt", "not a regular file"),
    ] {
        let result = command(directory.path())
            .args([
                input,
                "--model",
                "shakkelha",
                "--cache-dir",
                "model-cache",
                "-o",
                output,
            ])
            .output()
            .unwrap();
        failure(&result, 1, message);
        assert!(!directory.path().join("model-cache").exists());
        assert!(!directory.path().join(output).exists());
    }
}

#[test]
fn oversized_docx_is_rejected_from_metadata_before_reading_or_model_setup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("oversized.docx");
    let maximum = tashkeel::docx::Limits::default().max_archive_bytes;
    fs::File::create(&path)
        .unwrap()
        .set_len(u64::try_from(maximum).unwrap() + 1)
        .unwrap();
    let result = command(directory.path())
        .args([
            "oversized.docx",
            "--model",
            "shakkelha",
            "--cache-dir",
            "model-cache",
            "-o",
            "result.docx",
        ])
        .output()
        .unwrap();
    failure(&result, 1, "archive size limit");
    assert!(!directory.path().join("model-cache").exists());
    assert!(!directory.path().join("result.docx").exists());
    assert_eq!(
        fs::metadata(path).unwrap().len(),
        u64::try_from(maximum).unwrap() + 1
    );
}

#[test]
fn valid_docx_is_preserved_when_setup_fails_or_output_already_exists() {
    let directory = tempfile::tempdir().unwrap();
    let source = include_bytes!("fixtures/formatted.docx");
    fs::write(directory.path().join("book.docx"), source).unwrap();
    for (output, expected) in [("book.docx", "already exists"), ("result.docx", "offline:")] {
        let result = command(directory.path())
            .args([
                "book.docx",
                "--model",
                "shakkelha",
                "--offline",
                "--cache-dir",
                "model-cache",
                "-o",
                output,
            ])
            .output()
            .unwrap();
        failure(&result, 1, expected);
        assert_eq!(
            fs::read(directory.path().join("book.docx")).unwrap(),
            source
        );
        assert!(!directory.path().join("result.docx").exists());
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn an_existing_output_and_in_place_input_are_protected_before_downloads() {
    let directory = tempfile::tempdir().unwrap();
    let original = "\u{feff} السلام عليكم\r\n\r\n  ";
    fs::write(directory.path().join("input.txt"), original).unwrap();
    fs::write(directory.path().join("existing.txt"), "keep me").unwrap();
    for output in ["input.txt", "existing.txt"] {
        let result = command(directory.path())
            .args([
                "input.txt",
                "--model",
                "shakkelha",
                "--cache-dir",
                "model-cache",
                "-o",
                output,
            ])
            .output()
            .unwrap();
        failure(&result, 1, "already exists; use --force");
        assert!(!directory.path().join("model-cache").exists());
    }
    assert_eq!(
        fs::read_to_string(directory.path().join("input.txt")).unwrap(),
        original
    );
    assert_eq!(
        fs::read_to_string(directory.path().join("existing.txt")).unwrap(),
        "keep me"
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn force_does_not_truncate_an_output_when_model_setup_fails() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("input.txt"), "السلام\r\n").unwrap();
    let result = command(directory.path())
        .args([
            "input.txt",
            "--model",
            "shakkelha",
            "--offline",
            "--cache-dir",
            "model-cache",
            "-o",
            "input.txt",
            "--force",
            "--quiet",
        ])
        .output()
        .unwrap();
    failure(&result, 1, "offline:");
    assert_eq!(
        fs::read_to_string(directory.path().join("input.txt")).unwrap(),
        "السلام\r\n"
    );
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn unusable_output_parent_and_missing_runtime_fail_before_downloads() {
    let directory = tempfile::tempdir().unwrap();
    for (args, message) in [
        (
            vec!["-o", "missing/result.txt"],
            "parent directory must exist",
        ),
        (vec!["--ort-library", "missing.so"], "ONNX Runtime library"),
        (vec!["--ort-library", "."], "not a regular file"),
    ] {
        let result = command(directory.path())
            .args([
                "--text",
                "السلام",
                "--model",
                "shakkelha",
                "--cache-dir",
                "model-cache",
            ])
            .args(args)
            .output()
            .unwrap();
        failure(&result, 1, message);
        assert!(!directory.path().join("model-cache").exists());
    }
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn offline_and_local_model_errors_are_actionable_and_do_not_download() {
    let directory = tempfile::tempdir().unwrap();
    for (args, message) in [
        (vec!["--offline"], "offline:"),
        (vec!["--model-dir", "local-model"], "verifying --model-dir"),
        (
            vec!["--offline", "--model-dir", "local-model"],
            "verifying --model-dir",
        ),
    ] {
        let result = command(directory.path())
            .args([
                "--text",
                "السلام",
                "--model",
                "catt",
                "--cache-dir",
                "model-cache",
                "-q",
            ])
            .args(args)
            .output()
            .unwrap();
        failure(&result, 1, message);
        assert!(!stderr(&result).contains("missing-onnxruntime-library"));
        assert!(!directory.path().join("model-cache").exists());
    }
}

#[test]
fn corrupt_cached_download_is_reported_without_runtime_loading_or_data_loss() {
    let directory = tempfile::tempdir().unwrap();
    let cache = directory.path().join("model-cache");
    let model_dir = tashkeel::models::model_dir(tashkeel::Model::Shakkelha, &cache);
    fs::create_dir_all(&model_dir).unwrap();
    let model_file = model_dir.join("model.onnx");
    fs::write(&model_file, "bad model data").unwrap();
    let result = command(directory.path())
        .args([
            "models",
            "download",
            "--model",
            "shakkelha",
            "--cache-dir",
            "model-cache",
            "--quiet",
        ])
        .output()
        .unwrap();
    failure(&result, 1, "verifying shakkelha");
    assert!(!stderr(&result).contains("missing-onnxruntime-library"));
    assert_eq!(fs::read_to_string(model_file).unwrap(), "bad model data");
}

#[test]
fn stdin_is_explicit_utf8_text_and_never_an_implicit_input() {
    let directory = tempfile::tempdir().unwrap();
    for (input, message) in [
        (" السلام\r\n ".as_bytes(), "offline:"),
        (&[0xff][..], "UTF-8 text from stdin"),
    ] {
        let mut child = command(directory.path())
            .args([
                "-",
                "--model",
                "shakkelha",
                "--offline",
                "--cache-dir",
                "model-cache",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        failure(&child.wait_with_output().unwrap(), 1, message);
        assert!(!directory.path().join("model-cache").exists());
    }
}

#[test]
fn empty_direct_text_is_an_input_and_reaches_explicit_model_setup() {
    let directory = tempfile::tempdir().unwrap();
    let result = command(directory.path())
        .args([
            "--text",
            "",
            "--model",
            "shakkelha",
            "--offline",
            "--cache-dir",
            "model-cache",
        ])
        .output()
        .unwrap();
    failure(&result, 1, "offline:");
}

#[test]
fn completion_scripts_and_partial_command_protocol_need_no_setup() {
    let directory = tempfile::tempdir().unwrap();
    for shell in ["bash", "elvish", "fish", "nu", "powershell", "zsh"] {
        let output = command(directory.path())
            .args(["completions", shell])
            .output()
            .unwrap();
        success(&output);
        assert!(stdout(&output).contains("__complete_word__"));
    }
    for args in [
        vec!["completions", "invalid"],
        vec!["completions", "fish", "--dry-run"],
    ] {
        failure(
            &command(directory.path()).args(args).output().unwrap(),
            2,
            "error:",
        );
    }
    let result = command(directory.path())
        .args([
            "__complete_word__",
            "--shell",
            "bash",
            "--words",
            "tashkeel",
            "diacritize",
            "@missing.args",
            "--model",
            "sh",
        ])
        .output()
        .unwrap();
    success(&result);
    assert!(stdout(&result).contains("shakkelha"));
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn completion_installation_supports_preview_idempotence_and_foreign_file_protection() {
    let directory = tempfile::tempdir().unwrap();
    let target = directory
        .path()
        .join("config/fish/completions/tashkeel.fish");
    let install = |dry_run| {
        let mut command = command(directory.path());
        command.args(["completions", "fish", "--install"]);
        if dry_run {
            command.arg("--dry-run");
        }
        command.output().unwrap()
    };
    let preview = install(true);
    success(&preview);
    assert!(stdout(&preview).contains(&target.display().to_string()));
    assert!(!target.exists());
    success(&install(false));
    assert!(
        fs::read_to_string(&target)
            .unwrap()
            .contains("__complete_word__")
    );
    let repeated = install(false);
    success(&repeated);
    assert!(stdout(&repeated).contains("Already installed"));
    fs::write(&target, "# user maintained completions\n").unwrap();
    let result = install(false);
    assert_eq!(result.status.code(), Some(1), "{}", stderr(&result));
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "# user maintained completions\n"
    );
}

#[test]
fn response_files_support_nested_presets_and_report_cycles_and_bad_quoting() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("presets")).unwrap();
    fs::write(
        directory.path().join("presets/model.args"),
        "--model shakkelha --offline --cache-dir model-cache",
    )
    .unwrap();
    fs::write(
        directory.path().join("run.args"),
        "@presets/model.args --text ' السلام عليكم  '",
    )
    .unwrap();
    failure(
        &command(directory.path()).arg("@run.args").output().unwrap(),
        1,
        "offline:",
    );
    assert!(!directory.path().join("model-cache").exists());
    fs::write(directory.path().join("cycle.args"), "@cycle.args").unwrap();
    fs::write(directory.path().join("broken.args"), "'unclosed").unwrap();
    for file in ["cycle.args", "broken.args", "missing.args"] {
        failure(
            &command(directory.path())
                .arg(format!("@{file}"))
                .output()
                .unwrap(),
            2,
            file,
        );
    }
}

#[test]
fn literal_at_paths_are_not_expanded() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("@input.txt"), "السلام").unwrap();
    let result = command(directory.path())
        .args([
            "@@input.txt",
            "--model",
            "shakkelha",
            "--offline",
            "--cache-dir",
            "model-cache",
        ])
        .output()
        .unwrap();
    failure(&result, 1, "offline:");
}
