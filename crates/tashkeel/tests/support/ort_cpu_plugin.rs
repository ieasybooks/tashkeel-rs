//! Native Linux sentinel for automatic CPU plugin selection.

use std::{
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use ort::{
    environment::Environment,
    memory::DeviceType,
    session::{Session, builder::AutoDevicePolicy},
};

use super::{PLUGIN_SELECTED_MARKER, configured_path};

pub(super) fn register_and_check_control(host: &Environment, model: &Path) -> tempfile::TempDir {
    let directory = compile_plugin();
    host.register_ep_library("tashkeel-sentinel", directory.path().join("sentinel.so"))
        .unwrap();
    assert!(host.devices().any(|device| {
        device.ep().unwrap() == "TashkeelSentinelExecutionProvider"
            && device.hardware_device().ty() == DeviceType::CPU
    }));

    // Reproduce the previous builder configuration: disabling Rust default
    // providers does not prevent native PreferCPU from selecting this plugin.
    let error = Session::builder(host)
        .unwrap()
        .with_no_environment_execution_providers()
        .unwrap()
        .with_auto_device(AutoDevicePolicy::PreferCPU)
        .unwrap()
        .with_independent_thread_pool()
        .unwrap()
        .with_intra_threads(2)
        .unwrap()
        .with_inter_threads(1)
        .unwrap()
        .with_parallel_execution(false)
        .unwrap()
        .commit_from_file(model)
        .unwrap_err();
    assert!(
        error.to_string().contains(PLUGIN_SELECTED_MARKER),
        "{error}"
    );
    directory
}

fn compile_plugin() -> tempfile::TempDir {
    let include = configured_path("TASHKEEL_TEST_ORT_INCLUDE_DIR");
    let directory = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ort_cpu_plugin.c");
    // Write diagnostics to a file so a verbose compiler cannot block on a
    // full pipe while the parent enforces the timeout.
    let diagnostics = directory.path().join("compiler.log");
    let mut compiler = Command::new("cc")
        .args([
            "-std=c11", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", "-I",
        ])
        .arg(include)
        .arg(source)
        .arg("-o")
        .arg(directory.path().join("sentinel.so"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&diagnostics).unwrap())
        .spawn()
        .expect("native CPU plugin regression requires cc and the pinned ORT SDK headers");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = compiler.try_wait().unwrap() {
            assert!(
                status.success(),
                "{}",
                std::fs::read_to_string(diagnostics).unwrap()
            );
            break;
        }
        if Instant::now() >= deadline {
            compiler.kill().unwrap();
            compiler.wait().unwrap();
            panic!("native CPU plugin compiler exceeded 30 seconds");
        }
        thread::sleep(Duration::from_millis(20));
    }
    directory
}
