# Model conversion and validation

Validation date: 2026-10-26. Inference uses Rust `ort =2.0.0-rc.15`, dynamically loaded ONNX Runtime API 31, CPU execution, one intra-operation thread, one inter-operation thread, sequential graph execution, and exact-length batch-one inputs. No Python server, TensorFlow runtime, GPU, quantization, or padded batch is used by the library. Python is needed only to reproduce the conversion and independent validation below.

## Shakkelha artifact

The source is [AliOsm/shakkelha at commit 2214779456a905964d764dc4921fb70aa0d0aab6](https://github.com/AliOsm/shakkelha/tree/2214779456a905964d764dc4921fb70aa0d0aab6). The selected checkpoint is `models/rnn_models/3_normalized_model/big_data/lstm/avg_20.ckpt`, SHA-256 `46b18b13c3e3f9a45434d1ba70086e0ab6556456b6e3282b02bfb16ec4eed1ca`. The original CuDNN checkpoint is `models/rnn_models/3_normalized_model/big_data/avg_20.ckpt`, SHA-256 `d737aafeaa5b9ac277a306219ad30d425e103993351cbf288707af97fefe0aa8`.

The [focused exporter](../scripts/model-export/export_shakkelha.py) reads HDF5 weights, reorders Keras gates from `i,f,c,o` to ONNX `i,o,f,c`, represents both bidirectional recurrent layers with standard ONNX LSTM operators, and retains the embedding and three dense layers. It uses sigmoid gates to restore the original CuDNN semantics. The CPU checkpoint's serialized hard-sigmoid setting differs from those semantics. The exporter deliberately preserves the researched graph metadata and reproduces `model.onnx` byte for byte: SHA-256 `41ed03075e80847aaef2bfdb65f123b7f225ff6a179a507b1ec3e6152184fa71`. The release artifact did not change during this implementation.

Applying Keras's legacy CuDNN-to-LSTM input-kernel rearrangement, recurrent-kernel transpose, and summed bias conversion to the original GPU checkpoint produces all 19 CPU weight arrays exactly, with zero maximum absolute error. Embedding and dense arrays also match exactly. This verifies the checkpoint weight transformation; it is not live execution of the historical GPU stack. See [checkpoint-validation.json](../scripts/model-export/checkpoint-validation.json).

An independent NumPy recurrence and Python ONNX Runtime agreed on every predicted class for 30-, 100-, 500-, and 1,000-character samples. Maximum absolute probability differences were respectively `1.19e-7`, `1.19e-7`, `2.38e-7`, and `7.15e-7`. The Rust backend also exactly matched the saved Python ONNX Runtime classes at all 102 positions of the committed Shakkelha fixture, including SOS/EOS.

## Historical prediction differences and full evaluator

The sigmoid export is **not identical to the saved upstream predictions**. The earlier 200-line research sample differed at 9 of 45,917 base positions; hard-sigmoid differed at 234 positions. The completed 2,500-line comparison found 140 differing mark labels among 570,202 base positions, with 2,382 lines identical to the saved predictions. All comparisons preserved and aligned the same base text. Small floating-point differences between historical GPU and current CPU kernels are a possible explanation, but the cause has not been established by a live historical GPU comparison.

The full evaluation uses the pinned [arabic-text-diacritization test corpus and Fadel evaluator](https://github.com/AliOsm/arabic-text-diacritization/tree/e083579c03e489936200b69d792fd426bb576743), rather than presenting a raw sample label mismatch rate as published DER. Gold SHA-256 is `4e851ff836f0a178abb15d9f4a8bcf92748b77cc0d67030d9fae2fbe698baa12`; saved prediction SHA-256 is `4a88de364b538ca20e65e8d6b6c041eea37942509cbc4ef1ad19392482a8173f`; evaluator SHA-256 is `535b729c349965bfcb6863fc65c06a5465d3b900128ce19ce135998620e32fc8`. The original evaluator functions were executed with Fadel cleanup, all four case-ending/no-diacritic settings, and the embedded equivalent class list. Running it on the saved upstream predictions reproduces the upstream published rounded scores.

| Case endings | Include unmarked letters | Saved upstream DER / WER | Whole-line sigmoid ONNX DER / WER | Default Rust CLI DER / WER |
| --- | --- | --- | --- | --- |
| Yes | Yes | 1.69% / 5.09% | 1.68% / 5.08% | 1.74% / 5.26% |
| No | Yes | 1.34% / 2.91% | 1.34% / 2.90% | 1.38% / 3.01% |
| Yes | No | 1.95% / 4.89% | 1.94% / 4.87% | 2.01% / 5.05% |
| No | No | 1.54% / 2.83% | 1.53% / 2.83% | 1.59% / 2.93% |

The saved and whole-line columns use one complete source line per model call as upstream did. A separate actual-CLI run submitted the entire stripped 2,500-line document with the default 500-character chunking and one thread. It preserved all source characters and line structure exactly, but errors increased in all four settings compared with whole-line inference. This comparison does not isolate which pipeline difference caused the increase. Changing chunk boundaries changes context and can change predictions. These rounded corpus-specific measurements are not proof of output equivalence or a general accuracy guarantee. Reports: [whole-line validation](../scripts/model-export/validation-full.json) and [default CLI validation](../scripts/model-export/validation-cli.json).

## CATT encoder-only artifact and parity

CATT uses the original released encoder-only ONNX files, without re-exporting or modifying weights. The source/tokenizer reference is [abjadai/catt at commit 8d5330499feb85f6625e6af632141ed2ed6065fd](https://github.com/abjadai/catt/tree/8d5330499feb85f6625e6af632141ed2ed6065fd). The archive `eo_model_onnx.zip` has SHA-256 `5343bfdfda70c981b587811b79348b8dde91f65c0a551ca98f232ea5cb62ceac`. `encoder.onnx` has SHA-256 `437a99c032ec7de4249280efbc4dce2d0532f95c9fe8356debf24ce05a34ba6a`; `decoder.onnx` has SHA-256 `e6575bdba8623bf51a6d459fc4955e8a98aa3c6c2ffc5abffac26e883792bc80`. Here `decoder.onnx` is the encoder-only classification head, not an autoregressive decoder.

The Rust implementation exactly matches all input IDs, argmax labels, and decoded Arabic-only fixture strings captured with Python ONNX Runtime 1.31 at 100, 500, 1,000, and 1,024 positions, totaling 2,624 positions. It removes BOS/EOS before calling the encoder, uses an all-true `[1,1,N,N]` boolean attention mask because there is no padding, validates `[1,N,512]` encoder features, and validates `[1,N,18]` classifier outputs. The 1,024-position limit includes all normalized letters and internal spaces; a 1,025-position input is split before inference. Tests also cover expansions whose source scalar count fits but normalized length exceeds the model limit. CATT task accuracy and parity with its original PyTorch checkpoint were not re-evaluated here.

## Text preservation and scope

Only U+064B–U+0652 are removed and replaced. Source base characters, Latin text, numbers, punctuation, tatweel, other combining marks, BOMs, line endings, and whitespace are retained. Output is assembled against original byte offsets; it is never decoded from destructive normalized model input. Special output classes mean no mark and never remove or shift a source character.

CATT treats unsupported input characters as word separators, collapses separators for model input, ignores tatweel and dagger alif for model input, and maps alif-wasla to alif while retaining the original character in output. Arabic presentation forms are compatibility-normalized only for model input. A presentation form that becomes exactly one Arabic letter can receive a mark; a ligature that expands to several letters remains unmarked because internal marks cannot be represented without changing the original base character. Its expanded tokens still provide context to surrounding letters. These edge-case policies deliberately provide preservation and broader input handling; they are not an assertion of equality with upstream's destructive normalization. Shakkelha uses its original character vocabulary and maps unsupported characters to UNK without adding marks to them.

Chunking first recognizes sentence and paragraph boundaries, then prefers whitespace boundaries when a segment is too long, and finally splits overlong words between source Unicode scalars. `chunk_size` counts original scalars, including existing common marks, and defaults to 500. There is no whole-document or web 1,000-character limit. CATT additionally enforces its normalized position limit. Empty and non-Arabic chunks do not invoke native inference. Progress reports all chunks, beginning at `(0,total)`; cancellation between chunks returns `Error::Cancelled` without a partial result.

Standalone runtime lookup checks an explicit option, `ORT_DYLIB_PATH`, then the standard runtime library name beside the current executable. Windows and macOS require an explicit or adjacent library; other Unix systems also allow platform-loader lookup. Configured paths must resolve to existing files. One ONNX Runtime environment is retained per process, allowing multiple models and repeated loads; an attempt to select a different runtime path after initialization returns an error. Applications that already initialize `ort` can supply their existing environment through `Diacritizer::load_with_environment`, as described in [integration](integration.md#host-owned-ort-environment). Both loading paths use CPU sessions with independent thread pools. Model load checks tensor names, types, ranks, dynamic sequence axes, batch compatibility, feature widths, and class counts. Runtime output checks reject unexpected shapes and non-finite scores. The caller must verify model hashes before loading; the public model preparation helpers provide this verification.

## Reproduction

Use Python 3.12 and [requirements.txt](../scripts/model-export/requirements.txt) in a dedicated environment. Obtain the two checkpoints, gold corpus, saved predictions, and evaluator from the pinned commits above. The scripts accept local files, verify the checkpoint/model/evaluator hashes, and do not download or serve anything. The `--gpu-checkpoint` option verifies the weight conversion only. Keep the resulting large files outside the repository.

```sh
python scripts/model-export/export_shakkelha.py \
  --checkpoint /path/to/cpu/avg_20.ckpt \
  --gpu-checkpoint /path/to/gpu/avg_20.ckpt \
  --output /tmp/tashkeel-validation/model.onnx \
  --report /tmp/tashkeel-validation/checkpoint-validation.json

OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1 MKL_NUM_THREADS=1 \
python scripts/model-export/validate_shakkelha.py \
  --checkpoint /path/to/cpu/avg_20.ckpt \
  --model /tmp/tashkeel-validation/model.onnx \
  --gold /path/to/test.txt \
  --published /path/to/20-output.txt \
  --evaluator /path/to/diacritization_stat.py \
  --output /tmp/tashkeel-validation/predictions.txt \
  --report /tmp/tashkeel-validation/full-validation.json
```

Omitting `--limit` evaluates all 2,500 lines. `--limit 200` reproduces the focused saved-output comparison. The committed reference fixtures contain token IDs and predicted classes only, plus CATT reference text; no model weights are committed. Model-free tests require no native runtime or model downloads. To run the four opt-in library tests, place each model's files directly inside its directory and set:

```sh
export CARGO_BUILD_JOBS=1
export ORT_DYLIB_PATH=/absolute/path/to/libonnxruntime.so
export TASHKEEL_TEST_SHAKKELHA_DIR=/absolute/path/to/shakkelha
export TASHKEEL_TEST_CATT_DIR=/absolute/path/to/catt
mise x rust@1.99.1 -- cargo test -p tashkeel --lib --locked \
  inference::tests::real_ -- --ignored --test-threads=1
```

Reproduce the separate default-CLI evaluation using the Python standard-library-only script below. It checks pinned corpus, evaluator, model, and vocabulary hashes; verifies the executable's advertised defaults; and leaves large text artifacts under `target/validation/cli`. It does not download models or rebuild the binary.

```sh
python3 scripts/model-export/validate_cli.py \
  --binary target/release/tashkeel \
  --model-dir /path/to/shakkelha/v1 \
  --ort-library /path/to/libonnxruntime.so \
  --gold /path/to/test.txt \
  --evaluator /path/to/diacritization_stat.py \
  --report target/validation/cli/report.json
```

All four library tests passed with the parent's SHA-verified official Linux x86_64 runtime at `target/onnxruntime/x86_64-unknown-linux-gnu/libonnxruntime.so` and prepared model directories. The tests include exact fixture parity, preservation on mixed Arabic/Latin text, empty input, long documents, cancellation before and after a chunk, repeated loads of both models, and CATT's position-limit edges. The CLI test `real_models_preserve_text_from_arguments_stdin_and_files` also passed for both models, including actual UTF-8 files, stdin, exact whitespace/BOM preservation, and atomic file output. A separate relocated-binary check passed with the native library beside the executable, `ORT_DYLIB_PATH` unset, and `/` as the working directory. Broader DOCX/application checks are maintained alongside the CLI implementation. No macOS/Windows execution, live historical GPU equivalence, or general model accuracy guarantee is established by these tests.
