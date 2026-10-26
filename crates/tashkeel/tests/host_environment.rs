//! Host-owned ort integration runs in its own process: ort permits one environment.

// Preserve the reference fixtures' original mark ordering.
#![allow(clippy::unicode_not_nfc)]

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use ort::{environment::GlobalThreadPoolOptions, ep, logging::LogLevel, session::Session};
use tashkeel::{Diacritizer, Error, InferenceOptions, Model, models, text::strip_tashkeel};

// The first Python ONNX Runtime CPU fixtures in src/inference/fixtures.rs.
// Shakkelha's expected string renders the saved reference labels using its
// embedded vocabulary; CATT's string is also saved in the upstream fixture.
const SHAKKELHA_INPUT: &str = "اللغة العربية لغة جميلة وغنية، وهي وسيلة للتواصل بين الناس ونقل العلوم والمعارف. يقرأ الطالب كتابا م";
const SHAKKELHA_OUTPUT: &str = "اللُّغَةُ الْعَرَبِيَّةُ لُغَةٌ جَمِيلَةٌ وَغَنِيَّةٌ، وَهِيَ وَسِيلَةٌ لِلتَّوَاصُلِ بَيْنَ النَّاسِ وَنَقْلِ الْعُلُومِ وَالْمَعَارِفِ. يَقْرَأُ الطَّالِبُ كِتَابًا م";
const CATT_INPUT: &str = "سطا لصوص على منزل وزيرة الدفاع التشيلية مايا فرنانديز بعد ضرب ابنها وتهديد زوجها وفق ما أعلنته السلط";
const CATT_OUTPUT: &str = "سَطَا لُصُوصٌ عَلَى مَنْزِلِ وَزِيرَةِ الدِّفَاعِ التَّشَيُّلِيَّةِ مَايا فرنَانْدِيز بَعْدَ ضَرْبِ ابْنِهَا وَتَهْدِيدِ زَوْجِهَا وَفْقَ مَا أَعْلَنَتْهُ السَّلْطُ";

fn configured_path(variable: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(variable).unwrap_or_else(|| panic!("set {variable} for this opt-in test")),
    )
    .canonicalize()
    .unwrap()
}

fn check_output(engine: &mut Diacritizer, input: &str, expected: &str) {
    let output = engine.diacritize(input).unwrap();
    assert_eq!(output, expected);
    assert_eq!(strip_tashkeel(&output), input);
    assert_eq!(
        engine.diacritize("API 42\r\n🙂\t").unwrap(),
        "API 42\r\n🙂\t"
    );
}

fn host_environment(
    runtime: &std::path::Path,
    logs: &Arc<Mutex<Vec<String>>>,
) -> ort::environment::Environment {
    let captured = logs.clone();
    // init_from must precede GlobalThreadPoolOptions, which uses the native API.
    let builder = ort::init_from(runtime).unwrap();
    let pool = GlobalThreadPoolOptions::default()
        .with_intra_threads(3)
        .unwrap()
        .with_inter_threads(2)
        .unwrap()
        .with_spin_control(false)
        .unwrap();
    let host = builder
        .with_global_thread_pool(pool)
        // CPU is available without extra Cargo features. Changing its arena
        // setting is a sentinel for inheritance of any host default provider.
        .with_execution_providers([ep::CPU::default().with_arena_allocator(false).build()])
        .with_logger(Arc::new(move |_, _, _, _, message| {
            if message.starts_with("Session Options {")
                || message.contains("use_per_session_threads_")
                || message.contains("All nodes placed on")
            {
                captured.lock().unwrap().push(message.to_owned());
            }
        }))
        .build()
        .unwrap();
    host.set_log_level(LogLevel::Verbose);
    host
}

#[test]
#[ignore = "requires both pinned local models and CPU ONNX Runtime API 31"]
fn host_environment_keeps_cpu_sessions_and_lifetime() {
    let runtime = configured_path("ORT_DYLIB_PATH");
    let shakkelha_dir = configured_path("TASHKEEL_TEST_SHAKKELHA_DIR");
    let catt_dir = configured_path("TASHKEEL_TEST_CATT_DIR");
    models::verify(Model::Shakkelha, &shakkelha_dir).unwrap();
    models::verify(Model::Catt, &catt_dir).unwrap();
    let logs = Arc::new(Mutex::new(Vec::new()));
    let host = host_environment(&runtime, &logs);

    // Positive control: an ordinary host session inherits both the default
    // provider's arena setting and the global pools. The injected sessions
    // below must instead keep independent CPU/thread settings.
    let control = Session::builder(&host)
        .unwrap()
        .commit_from_file(shakkelha_dir.join("model.onnx"))
        .unwrap();
    drop(control);
    assert!(logs.lock().unwrap().iter().any(|message| {
        message.starts_with("Session Options {")
            && message.contains("enable_cpu_mem_arena:0")
            && message.contains("use_per_session_threads:0")
    }));
    logs.lock().unwrap().clear();

    // Even the same native path is ambiguous here and must be rejected rather
    // than silently ignored; the host already selected the runtime.
    let conflict = Diacritizer::load_with_environment(
        Model::Shakkelha,
        &shakkelha_dir,
        InferenceOptions {
            runtime_path: Some(runtime),
            ..InferenceOptions::default()
        },
        &host,
    );
    assert!(
        matches!(conflict, Err(Error::InvalidInput(message)) if message.contains("runtime_path"))
    );

    let options = InferenceOptions {
        threads: 2,
        ..InferenceOptions::default()
    };
    let mut shakkelha = Diacritizer::load_with_environment(
        Model::Shakkelha,
        &shakkelha_dir,
        options.clone(),
        &host,
    )
    .unwrap();
    let mut catt = Diacritizer::load_with_environment(
        Model::Catt,
        &catt_dir,
        InferenceOptions::default(),
        &host,
    )
    .unwrap();
    check_output(&mut shakkelha, SHAKKELHA_INPUT, SHAKKELHA_OUTPUT);
    check_output(&mut catt, CATT_INPUT, CATT_OUTPUT);

    // Releasing an engine and the original host handle does not prevent later
    // loads from a retained clone, or invalidate other engines.
    let retained = host.clone();
    drop(host);
    drop(shakkelha);
    let mut replacement =
        Diacritizer::load_with_environment(Model::Shakkelha, &shakkelha_dir, options, &retained)
            .unwrap();
    drop(retained);
    check_output(&mut replacement, SHAKKELHA_INPUT, SHAKKELHA_OUTPUT);
    check_output(&mut catt, CATT_INPUT, CATT_OUTPUT);

    assert_session_settings(&logs.lock().unwrap());
}

fn assert_session_settings(logs: &[String]) {
    // Inspect the pinned native runtime's own session diagnostics, not just
    // successful inference: global pools would ignore per-session threads.
    let settings: Vec<_> = logs
        .iter()
        .filter(|message| message.starts_with("Session Options {"))
        .collect();
    assert_eq!(settings.len(), 4, "{logs:#?}");
    for (message, threads) in settings.iter().zip([2, 1, 1, 2]) {
        assert!(message.contains("use_per_session_threads:1"), "{message}");
        assert!(message.contains("enable_cpu_mem_arena:1"), "{message}");
        assert!(message.contains("execution_mode:0"), "{message}");
        assert!(message.contains("ep_selection_policy:1"), "{message}");
        assert!(
            message.contains(&format!(
                "intra_op_param:OrtThreadPoolParams {{ thread_pool_size: {threads}"
            )),
            "{message}"
        );
        assert!(
            message.contains("inter_op_param:OrtThreadPoolParams { thread_pool_size: 1"),
            "{message}"
        );
    }
    assert_eq!(
        logs.iter()
            .filter(|message| message.contains("use_per_session_threads_ is true"))
            .count(),
        4,
        "{logs:#?}"
    );
    let placements: Vec<_> = logs
        .iter()
        .filter(|message| message.contains("All nodes placed on"))
        .collect();
    assert_eq!(placements.len(), 4, "{logs:#?}");
    assert!(
        placements
            .iter()
            .all(|message| message.contains("[CPUExecutionProvider]"))
    );
}
