# Models

Both backends run FP32, batch one, on the CPU through ONNX Runtime. Select a model explicitly; there is no silent backend fallback. The default is one inference thread and up to 500 source Unicode scalars per chunk. Sentence, paragraph, and word boundaries are preferred; context changes at chunk boundaries can change predictions. The web application's intended 1,000-character limit belongs at that interface, not in this document-capable library.

## Shakkelha

The source is [AliOsm/shakkelha](https://github.com/AliOsm/shakkelha/tree/2214779456a905964d764dc4921fb70aa0d0aab6), normalized RNN, big training data, averaged last 20 epochs. Averaging produces one checkpoint; this is not an ensemble. The model has two bidirectional LSTM layers and approximately 2.69 million parameters. The related `arabic-text-diacritization` repository provides the benchmark and evaluation utilities.

The original CPU HDF5 checkpoint is `models/rnn_models/3_normalized_model/big_data/lstm/avg_20.ckpt`, SHA-256 `46b18b13c3e3f9a45434d1ba70086e0ab6556456b6e3282b02bfb16ec4eed1ca`. Its serialized recurrence uses hard-sigmoid, whereas the training CuDNN model uses sigmoid. Tashkeel uses the researched sigmoid conversion, which is closer to saved upstream predictions. Reproduction tooling is under `scripts/model-export/`.

The converted `model.onnx` is 10,777,454 bytes, SHA-256 `41ed03075e80847aaef2bfdb65f123b7f225ff6a179a507b1ec3e6152184fa71`. It is distributed as `shakkelha_normalized_big_avg20_sigmoid.onnx` in the repository's `models-v1` artifact release. Its input vocabulary and class map are embedded in the crate from the corresponding upstream model.

**Validation status:** the conversion matches an independent NumPy recurrence, and Rust matches Python ONNX Runtime using this graph. All 19 weight arrays match the original GPU checkpoint after Keras conversion. On all 2,500 test lines, whole-line sigmoid inference differs from the saved upstream output at 140 of 570,202 base-character positions. The original evaluator reports 1.68% DER / 5.08% WER for whole-line ONNX, 1.69% / 5.09% for saved upstream output, and 1.74% / 5.26% for the actual Rust CLI with default chunking, including case endings and unmarked letters. The default pipeline therefore has slightly higher errors on this corpus. These measurements do not establish exact historical GPU equivalence or general accuracy. See [conversion validation](model-validation.md) for the full results and reproduction commands.

The repository and its committed checkpoint are MIT licensed. Preserve `licenses/Shakkelha-MIT` when redistributing the conversion.

## CATT encoder-only

The source is [abjadai/catt](https://github.com/abjadai/catt/tree/8d5330499feb85f6625e6af632141ed2ed6065fd), using the upstream [v2 `eo_model_onnx.zip`](https://github.com/abjadai/catt/releases/tag/v2). The release identifies the export as the `best_eo_mlm_ns_epoch_193.pt` noisy-student checkpoint. The graph has six transformer encoder layers, approximately 18.93 million parameters, and a parallel linear classifier. The file called `decoder.onnx` is that classifier, not the autoregressive CATT encoder-decoder model.

- Archive: 72,109,998 bytes; SHA-256 `5343bfdfda70c981b587811b79348b8dde91f65c0a551ca98f232ea5cb62ceac`.
- `encoder.onnx`: 77,915,147 bytes; SHA-256 `437a99c032ec7de4249280efbc4dce2d0532f95c9fe8356debf24ce05a34ba6a`.
- `decoder.onnx`: 37,294 bytes; SHA-256 `e6575bdba8623bf51a6d459fc4955e8a98aa3c6c2ffc5abffac26e883792bc80`.

The exported positional table supports at most 1,024 normalized positions. The implementation separately bounds normalized token counts and source-character chunks. Upstream preprocessing deletes punctuation, numbers, Latin text, and whitespace structure; tashkeel instead maps predictions onto the original source. Its output therefore intentionally preserves content that the upstream wrapper drops. Unicode normalization can expand one source character into multiple model tokens, which requires special handling during reconstruction.

Rust/ONNX parity is checked against fixtures from the same released graphs. PyTorch checkpoint-to-ONNX numerical parity has not been independently established. The upstream repository currently declares Apache-2.0 and explicitly announces the change from its former noncommercial license; the ONNX archive carries no separate weight-specific notice. Preserve `licenses/CATT-Apache-2.0` and upstream attribution with distributed model artifacts.

## Cache and offline use

`models::ensure` downloads into temporary storage, verifies size and SHA-256, and atomically publishes a complete directory. Existing cache files are rehashed, so same-size corruption is detected. A corrupt directory produces an error and is not silently deleted. Remove or relocate that directory explicitly to download a fresh copy.

Cache directories are `<cache-root>/shakkelha/v1/` and `<cache-root>/catt/v1/`. Shakkelha expects `model.onnx`; CATT expects `encoder.onnx` and `decoder.onnx`. `--model-dir` accepts this file layout and validates the pinned files without attempting network access. `--offline` only accepts a complete verified cache. Model files are never included in the Git repository or the published Rust crate.

Ctrl-C exits promptly while the CLI waits for input, verifies models, downloads weights, or loads the runtime. Inference stops between chunks; a second Ctrl-C forces immediate exit. Interrupted downloads can leave private `.download-*` staging directories beneath the model cache; these are never accepted as a published model and can be removed when no download is running. Forced termination can also skip temporary-file cleanup. Requested output files are published only after a complete transformation.
