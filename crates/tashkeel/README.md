# tashkeel

CPU Arabic diacritization with Shakkelha and CATT encoder-only, preserving original text and DOCX structure. This is the reusable library behind `tashkeel-cli`.

```rust,no_run
use std::path::Path;
use tashkeel::{Diacritizer, InferenceOptions, Model, models};

let directory = models::ensure(Model::Shakkelha, Path::new("model-cache"))?;
let mut engine = Diacritizer::load(Model::Shakkelha, &directory, InferenceOptions::default())?;
let output = engine.diacritize("اللغة العربية جميلة")?;
# Ok::<(), tashkeel::Error>(())
```

Inference needs CPU ONNX Runtime 1.31.0, discovered using `InferenceOptions::runtime_path`, `ORT_DYLIB_PATH`, the executable directory, or the Unix platform loader. Windows requires an explicit or adjacent DLL and the Microsoft Visual C++ v14 x64 Redistributable. Keep sessions alive, use a bounded blocking worker in async applications, and pass a progress callback for cancellation between chunks. Default inference uses one CPU thread and chunks up to 500 Unicode scalars with a separate normalized-token limit.

The Shakkelha sigmoid conversion is numerically validated against an independent recurrence but does not exactly reproduce every saved upstream prediction. Read the [model documentation](https://github.com/ieasybooks/tashkeel-rs/blob/main/docs/models.md) for provenance, limitations, and licenses. The [repository](https://github.com/ieasybooks/tashkeel-rs) documents runtime setup, DOCX support, and the CLI.

Tokenizer metadata and fixture attribution are included in [third-party notices](THIRD-PARTY-NOTICES.md).
