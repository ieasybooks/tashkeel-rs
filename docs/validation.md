# Validation

## Model-free checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny --locked check
cargo about generate --locked --all-features --workspace --fail about.hbs --output-file /tmp/tashkeel-notices.txt
actionlint
python3 .github/scripts/package-docs.py /tmp/tashkeel-release-docs
sh scripts/cli-docs.sh
git diff --exit-code -- docs/cli.md docs/tashkeel.1
```

The normal test suite uses local fixtures and a local HTTP test server, without downloading weights or loading the native runtime. It exercises text reconstruction, token limits, cache corruption checks, DOCX structure preservation, invalid inputs, CLI help/completions, and output collision/cancellation behavior. Dependency advisory exceptions are explained in [dependencies](dependencies.md).

## Real model checks

Provision CPU ONNX Runtime with `.github/scripts/onnxruntime.py --include-headers`, then download both models with the CLI. Set `ORT_DYLIB_PATH` to the helper's printed library path, `TASHKEEL_TEST_ORT_INCLUDE_DIR` to the adjacent `include/` directory, and point the following variables at directories containing the pinned model files. The Linux host-environment regression compiles a small native provider fixture with `cc`; these headers and a C compiler are test dependencies only.

```sh
export TASHKEEL_TEST_SHAKKELHA_DIR=/absolute/path/to/shakkelha/v1
export TASHKEEL_TEST_CATT_DIR=/absolute/path/to/catt/v1
export TASHKEEL_TEST_ORT_INCLUDE_DIR=/absolute/path/to/onnxruntime/include
cargo test --workspace --locked -- --ignored --test-threads=1
```

This workspace-wide command runs the ignored library and integration tests, including the separate host-environment test binary. The library checks cover fixture parity, repeated model loads, chunk boundaries, and cancellation. The CLI checks cover both models with direct text, stdin, TXT, and DOCX; UTF-8 BOMs, CRLF, whitespace, and final-newline preservation; offline local files; and cancellation without publishing partial output. Inference-specific tests and checkpoint conversion evidence are documented in [model validation](model-validation.md).

`python3 scripts/smoke-models.py /absolute/path/to/tashkeel` exercises downloads, real CPU inference, original-text preservation, and repeatable offline execution. It changes the child process's working directory to catch accidental assumptions about the checkout or runtime location. Set `TASHKEEL_SMOKE_CACHE` to reuse a cache between runs. CI and extracted release archives run this smoke check.

Native-model CI retains this cache under the runner's temporary directory. After the smoke check downloads both models, Linux x64 runs the workspace-wide ignored tests with `TASHKEEL_TEST_SHAKKELHA_DIR=$TASHKEEL_SMOKE_CACHE/shakkelha/v1`, `TASHKEEL_TEST_CATT_DIR=$TASHKEEL_SMOKE_CACHE/catt/v1`, and the runtime helper's `ORT_DYLIB_PATH` and `TASHKEEL_TEST_ORT_INCLUDE_DIR`. The helper provisions the fixture's C headers from the same checksum-verified archive. The smoke step unsets the runtime variable only in its own shell to exercise the adjacent-library lookup; the later test step inherits the helper's setting. This reuses the verified downloads and includes new ignored integration test binaries automatically.

After their smoke checks, the native Apple and Windows jobs run `.github/scripts/macos-loader-regression.py` and `.github/scripts/windows-loader-regression.py`, respectively, with `--binary` pointing to the relocated executable and `--model-dir` pointing to the same cached `shakkelha/v1` directory. Keep the Windows unrestricted-load, rejected-CWD-dependency, and accepted-adjacent-dependency stages together. These native jobs must pass before claiming platform-specific loader behavior; configuring CI alone does not establish it.

## Offline release documentation

[The documentation packager](../.github/scripts/package-docs.py) copies the original README, complete `docs/` tree, [release guide](../.github/RELEASING.md), licenses, and the small scripts, benchmark example, tokenizer data, and validation reports they reference, retaining their repository-relative paths. Model weights, caches, and build outputs are excluded. Cargo development, benchmark, and publishing commands still require a source checkout and their documented dependencies.

The model-free CI job stages these files and verifies that relative Markdown file links stay within the bundle and resolve, including links from the library notices to their license texts. The release job repeats the check after extracting each archive using `python3 .github/scripts/package-docs.py --check /path/to/extracted/tashkeel-VERSION-TARGET`. The check needs no network, native runtime, or model downloads; external upstream URLs and heading fragments are outside its scope.

## Compatibility boundaries

XML/package fixture checks establish that supported DOCX transformations preserve source structure and untouched bytes. They do not establish identical rendering or universal compatibility with every Word or LibreOffice version. Diacritics can change wrapping, line height, and pagination. Unsupported or ambiguous document structures are documented in [DOCX behavior](docx.md).

Rust/Python ONNX parity verifies use of the same graph. It does not independently establish that an exported graph reproduces the original checkpoint, that a published accuracy metric transfers to chunked documents, or that both models have comparable benchmark scores. Keep those claims separate from runtime correctness.

## Preliminary CPU timings

The release-mode [benchmark example](../crates/tashkeel/examples/benchmark.rs) exercises the persistent-session text path, including chunking, tokenization, native inference, and source reconstruction. On the Intel Core i5-8500, ONNX Runtime 1.31.0, one thread, and default 500-character chunks, three warm repetitions after one warmup gave these medians on 2026-10-26:

| Input characters | Shakkelha | CATT encoder-only |
| --- | --- | --- |
| 100 | 19 ms | 64 ms |
| 500 | 78 ms | 465 ms |
| 1,000 | 152 ms | 725 ms |
| 10,000 | 1.71 s | 22.09 s |

These are shared-host feasibility measurements, with package compilation overlapping. Host load rose from about 7 to 19 during CATT's run, so especially its document timing does not establish isolated throughput or linear scaling. Downloads, queueing, TXT/DOCX I/O, and session loading are excluded. Session loading was 83 ms for Shakkelha and 1.20 s for CATT in these processes. Full samples and load averages are retained in [Shakkelha results](benchmarks/shakkelha-cpu.json) and [CATT results](benchmarks/catt-cpu.json). No p95 or production latency claim follows from three samples.

With a verified model directory and `ORT_DYLIB_PATH` configured, reproduce the text benchmark using `cargo run --release --locked -p tashkeel --example benchmark -- shakkelha /path/to/shakkelha/v1`, then repeat with `catt` and its directory. Run on an idle machine for deployment sizing.
