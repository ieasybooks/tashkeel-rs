# tashkeel

Arabic diacritization on the CPU, as a Rust library and command-line tool. Supports Shakkelha and CATT encoder-only, UTF-8 text, and DOCX documents.

The `tashkeel` command processes direct text, stdin, one UTF-8 TXT file, or one DOCX file. Original letters, punctuation, numbers, and whitespace are preserved; the eight common Arabic vowel/shadda marks are replaced. DOCX processing uses `docxml` to edit text inside the original document structure.

## Install

Download the archive for your platform from the [latest release](https://github.com/ieasybooks/tashkeel-rs/releases/latest), extract it, and put `tashkeel` on your `PATH`. Keep the included ONNX Runtime library beside the executable. Archives support Linux x86_64/arm64, macOS arm64 and Windows x86_64. Windows also needs the [Microsoft Visual C++ v14 x64 Redistributable](https://aka.ms/vc14/vc_redist.x64.exe).

Or install with Cargo (Rust 1.99 or newer):

```sh
cargo install tashkeel-cli --locked
```

Cargo installations need CPU [ONNX Runtime 1.31.0](https://github.com/microsoft/onnxruntime/releases/tag/v1.31.0): pass `--ort-library PATH`, set `ORT_DYLIB_PATH`, or put the library beside `tashkeel`. From a checkout, `python3 .github/scripts/onnxruntime.py` downloads and verifies it and prints its path. Python is only used by this helper; inference runs entirely in Rust and native ONNX Runtime. See [runtime setup](docs/runtime.md) for other platforms and manual installation.

## Usage

```sh
tashkeel --model shakkelha --text 'اللغة العربية جميلة'
tashkeel --model catt input.txt -o output.txt
tashkeel --model shakkelha input.docx -o output.docx
printf 'اللغة العربية جميلة' | tashkeel --model shakkelha -

tashkeel models list
tashkeel models download --model catt
tashkeel --model catt --offline --text 'هذا مثال'
```

Model selection is explicit. The selected model downloads on first use and is cached with SHA-256 verification. Use `--offline` to prohibit downloads, `--model-dir DIR` for pinned files in an explicit directory, or `--cache-dir DIR` to choose the cache location. Existing output files require `--force`; input files are never implicitly overwritten.

Shakkelha uses the normalized-big averaged-20 sigmoid conversion. Its numerical runtime behavior and full-corpus scores are validated, although some predictions differ from the saved upstream output. CATT uses the upstream encoder-only noisy-student ONNX release. See [model provenance and limitations](docs/models.md) before comparing their accuracy. Long documents are processed in bounded chunks; ordinary Word layout may reflow after adding marks. Legacy `.doc` is unsupported. See [DOCX behavior](docs/docx.md).

## Library

Add the library to an application:

```sh
cargo add tashkeel
```

```rust,no_run
use std::path::Path;
use tashkeel::{Diacritizer, InferenceOptions, Model, models};

let directory = models::ensure(Model::Catt, Path::new("model-cache"))?;
let mut engine = Diacritizer::load(Model::Catt, &directory, InferenceOptions::default())?;
println!("{}", engine.diacritize("اللغة العربية جميلة")?);
let document = std::fs::read("input.docx")?;
let result = tashkeel::docx::transform(&document, |paragraph| engine.diacritize(paragraph))?;
std::fs::write("output.docx", result)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Inference is synchronous and needs the same ONNX Runtime library as the CLI. Keep an engine alive across requests. Applications that already initialize `ort` can use `Diacritizer::load_with_environment` to share their existing environment. Async servers should run inference and model setup on bounded blocking workers; see [integration](docs/integration.md) and the [API documentation](https://docs.rs/tashkeel).

## Documentation

- [Models](docs/models.md): provenance, limitations, caching and offline use.
- [Model conversion](docs/model-validation.md): how the pinned artifacts were produced and checked against upstream.
- [DOCX preservation](docs/docx.md): what is retained, how text is traversed, and resource limits.
- [Runtime setup](docs/runtime.md): ONNX Runtime discovery, source installation and release archives.
- [Integration](docs/integration.md): servers, blocking workers and shared `ort` environments.
- [Validation](docs/validation.md): model-free and real-model checks, compatibility and CPU timings.
- [CLI reference](docs/cli.md): every option, example and exit code. Also available through `tashkeel --help`.
- [Man page](docs/tashkeel.1): included in release archives.
- API documentation: [`tashkeel`](https://docs.rs/tashkeel).

Install shell completions, then follow any printed setup instructions (replace `zsh` with your shell):

```sh
tashkeel completions zsh --install
```

## Development

Install the configured tools with mise, then run the checks from the repository root:

```sh
mise install
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny --locked check
```

Model-free tests need neither native ONNX Runtime nor model downloads; real-model and packaging checks are described in [validation](docs/validation.md). After changing CLI declarations, regenerate the CLI reference and man page with `sh scripts/cli-docs.sh`; CI checks that they are current. See [AGENTS.md](https://github.com/ieasybooks/tashkeel-rs/blob/main/AGENTS.md) for project conventions and the [release workflow](.github/RELEASING.md) for releases.

## License

MIT. Model and dependency notices are in [licenses](licenses/) and [dependency notes](docs/dependencies.md).
