# tashkeel-rs

Arabic diacritization on the CPU, as a Rust library and command-line tool. Supports Shakkelha and CATT encoder-only, UTF-8 text, and DOCX documents.

The `tashkeel` library provides reusable, synchronous inference. The `tashkeel` command processes direct text, stdin, one UTF-8 TXT file, or one DOCX file. Original letters, punctuation, numbers, and whitespace are preserved; the eight common Arabic vowel/shadda marks are replaced. DOCX processing uses `docxml` to edit text inside the original document structure.

## Build

Use the pinned Rust toolchain (1.99.1):

```sh
cargo build --release --locked
python3 .github/scripts/onnxruntime.py
export ORT_DYLIB_PATH="$PWD/target/onnxruntime/x86_64-unknown-linux-gnu/libonnxruntime.so"
```

The runtime setup command prints the correct library path for the host. The export above is the Linux x64 example. Python is only used by this setup helper; inference runs entirely in Rust and native ONNX Runtime. Release archives bundle the native library next to the executable. See [runtime setup](docs/runtime.md) for other platforms and manual installation.

## Use

```sh
tashkeel --model shakkelha --text 'اللغة العربية جميلة'
tashkeel --model catt input.txt -o output.txt
tashkeel --model shakkelha input.docx -o output.docx
printf 'اللغة العربية جميلة' | tashkeel --model shakkelha -

tashkeel models list
tashkeel models download --model catt
tashkeel --model catt --offline --text 'هذا مثال'
tashkeel completions zsh --install
```

Use `target/release/tashkeel` after a local build, or place that executable on `PATH`. The selected model downloads on first use and is cached with SHA-256 verification. Use `--offline` to prohibit downloads, `--model-dir DIR` for pinned files in an explicit directory, or `--cache-dir DIR` to choose the cache location. Existing output files require `--force`; input files are never implicitly overwritten. Model selection is explicit.

Shakkelha uses the normalized-big averaged-20 sigmoid conversion. Its numerical runtime behavior and full-corpus scores are validated, although some predictions differ from the saved upstream output. CATT uses the upstream encoder-only noisy-student ONNX release. See [model provenance and limitations](docs/models.md) before comparing their accuracy. Long documents are processed in bounded chunks; ordinary Word layout may reflow after adding marks. Legacy `.doc` is unsupported. See [DOCX behavior](docs/docx.md).

## Library

```rust,no_run
use std::path::Path;
use tashkeel::{Diacritizer, InferenceOptions, Model, models};

let directory = models::ensure(Model::Catt, Path::new("model-cache"))?;
let mut engine = Diacritizer::load(Model::Catt, &directory, InferenceOptions::default())?;
let text = engine.diacritize("اللغة العربية جميلة")?;
let document = std::fs::read("input.docx")?;
let result = tashkeel::docx::transform(&document, |paragraph| engine.diacritize(paragraph))?;
std::fs::write("output.docx", result)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Keep an engine alive across requests. Async servers should run inference and model setup on bounded blocking workers; see [integration](docs/integration.md).

## Development

```sh
mise install
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny --locked check
```

Model-free tests need neither native ONNX Runtime nor model downloads. Real model and packaging checks are described in [validation](docs/validation.md). Detailed flags: [CLI reference](docs/cli.md). Releases: [release workflow](.github/RELEASING.md). Code is MIT licensed; model and dependency notices are in [licenses](licenses/) and [dependency notes](docs/dependencies.md).
