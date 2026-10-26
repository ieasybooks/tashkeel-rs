# Application integration

Create and retain a `Diacritizer` for each inference worker. Loading on every request adds latency and duplicates native allocations. Begin with one blocking worker, one inference thread, and batch one, then measure latency and memory before increasing concurrency. The library methods are synchronous; do not execute them directly on a Tokio executor thread.

Use `models::ensure` during setup or warmup, outside request-critical work, and call `Diacritizer::load` with the resulting directory. Persistent sessions do not spawn Python. The caller controls storage, input limits, model selection, and the native library location.

## Host-owned ort environment

If the host application uses ONNX Runtime for other models, initialize its environment first and pass a reference to `Diacritizer::load_with_environment(model, directory, options, &environment)`. The pinned `ort` binding allows only one environment creation per process; standalone `Diacritizer::load` cannot discover or reuse an environment created by the host. Use the same exact binding version and features in the host's `Cargo.toml`:

```toml
ort = { version = "=2.0.0-rc.15", default-features = false, features = ["std", "load-dynamic", "api-31"] }
```

```rust,no_run
use std::path::Path;
use tashkeel::{Diacritizer, InferenceOptions, Model, models};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Linux example: the host chooses and loads its trusted native runtime.
    let environment = ort::init_from("/opt/my-app/libonnxruntime.so")?
        .with_name("my-app")
        .build()?;
    let directory = Path::new("/opt/my-app/models/catt");
    models::verify(Model::Catt, directory)?;
    let mut engine = Diacritizer::load_with_environment(
        Model::Catt,
        directory,
        InferenceOptions::default(), // runtime_path is None
        &environment,
    )?;
    drop(environment); // The engine's sessions retain shared ownership.
    let output = engine.diacritize("اللغة العربية جميلة")?;
    println!("{output}");
    Ok(())
}
```

The host owns native runtime selection and trust, including dependency search; this entry point does not call the standalone loader or its Windows restricted preload. `runtime_path` must be `None`; even a path matching the host's library is rejected rather than silently ignored. `ORT_DYLIB_PATH` and adjacent libraries are irrelevant to the injected loading call because the environment already exists. Loading still checks model shapes and types, while the caller verifies pinned hashes with `models::verify` or `models::ensure`. Serving requires no Python.

Create multiple engines from the same environment reference or its clones. Each engine retains the environment through its sessions; dropping the original handle does not invalidate existing engines. Retain a clone to create more engines later. Dropping every handle does not reset the pinned binding's singleton, so do not attempt to create another environment afterward or mix host initialization with standalone `load`.

Injected sessions remain CPU-only: they opt out of the environment's default execution providers and override `ort`'s automatic device selection with `AutoDevicePolicy::PreferCPU`. They also opt out of global thread pools, use independent session pools with `InferenceOptions::threads` intra-operation threads, one inter-operation thread, sequential graph execution, and no spinning. A host GPU provider or differently sized global pool therefore cannot silently change the engine's resource settings. The host's logging and telemetry configuration remains in effect, and its other sessions are unaffected. Account for all session pools when bounding worker concurrency; CATT has two sessions per engine.

The opt-in `host_environment` integration-test binary uses its own process so the environment singleton cannot clash with standalone tests. With the pinned CPU runtime and both verified model directories available, run:

```sh
ORT_DYLIB_PATH=/absolute/path/to/libonnxruntime.so \
TASHKEEL_TEST_SHAKKELHA_DIR=/absolute/path/to/shakkelha \
TASHKEEL_TEST_CATT_DIR=/absolute/path/to/catt \
cargo test -p tashkeel --test host_environment --locked -- --ignored
```

The test initializes the host first, checks both models against saved CPU reference output, rejects a conflicting `runtime_path`, and exercises multiple engines after host handles are dropped. A host CPU provider with altered arena settings and a differently sized global pool form the inheritance control; native diagnostics must show that injected sessions instead preserve their own providers, CPU placement, and thread settings. This tests provider inheritance without requiring GPU hardware or extra GPU Cargo features.

## Request and document handling

`diacritize_with_progress` invokes a callback between bounded chunks. Return `ControlFlow::Break(())` to cancel. Cancellation is cooperative: allow the current native call to finish and retain the worker/queue slot until it does. Do not pretend a detached blocking task has stopped when an async request is cancelled. Progress contains completed and total chunk counts for that text operation; document paragraphs may start separate operations.

`docx::transform` accepts a closure over paragraph text. Reuse the same engine inside it and propagate cancellation/errors. Keep the result in temporary storage until processing completes, then publish the output atomically. Review [DOCX limits](docx.md) before accepting document uploads.

The intended `arlang.tools` interface is a maximum of 1,000 characters for interactive text and one TXT or DOCX upload. Count characters as Unicode scalars rather than UTF-8 bytes, and apply separate file/decompression limits. The Rust library intentionally supports longer documents and has no 1,000-character total-input restriction. Model normalization, source reconstruction, and the CATT normalized-position limit are internal concerns; the application's user-facing output remains the original document with updated Arabic marks.
