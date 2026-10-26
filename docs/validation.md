# Validation

## Model-free checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny --locked check
cargo about generate --locked --all-features --workspace --fail about.hbs --output-file /tmp/tashkeel-notices.txt
actionlint
sh scripts/cli-docs.sh
git diff --exit-code -- docs/cli.md docs/tashkeel.1
```

The normal test suite uses local fixtures and a local HTTP test server, without downloading weights or loading the native runtime. It exercises text reconstruction, token limits, cache corruption checks, DOCX structure preservation, invalid inputs, CLI help/completions, and output collision/cancellation behavior. Dependency advisory exceptions are explained in [dependencies](dependencies.md).

## Real model checks

Provision CPU ONNX Runtime with `.github/scripts/onnxruntime.py`, then download both models with the CLI. Set `ORT_DYLIB_PATH` to the helper's printed library path and point the following variables at directories containing the pinned model files:

```sh
export TASHKEEL_TEST_SHAKKELHA_DIR=/absolute/path/to/shakkelha/v1
export TASHKEEL_TEST_CATT_DIR=/absolute/path/to/catt/v1
cargo test --test real_cli -- --ignored --test-threads=1
```

These opt-in CLI checks cover both models with direct text, stdin, TXT, and DOCX; UTF-8 BOMs, CRLF, whitespace, and final-newline preservation; offline local files; and cancellation without publishing partial output. Inference-specific tests and checkpoint conversion evidence are documented in [model validation](model-validation.md).

`python3 scripts/smoke-models.py /absolute/path/to/tashkeel` exercises downloads, real CPU inference, original-text preservation, and repeatable offline execution. It changes the child process's working directory to catch accidental assumptions about the checkout or runtime location. Set `TASHKEEL_SMOKE_CACHE` to reuse a cache between runs. CI and extracted release archives run this smoke check.

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
