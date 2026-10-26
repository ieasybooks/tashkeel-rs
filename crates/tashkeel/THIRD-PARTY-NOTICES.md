# Model metadata and notices

The library includes tokenizer mappings and numerical reference fixtures derived from the following upstream projects. Model weights are downloaded separately and retain their own upstream notices.

- [AliOsm/Shakkelha](https://github.com/AliOsm/shakkelha/tree/2214779456a905964d764dc4921fb70aa0d0aab6): character/diacritic mappings in `data/shakkelha.json` and corresponding reference fixtures. MIT; the original copyright and permission notice are in [licenses/Shakkelha-MIT](licenses/Shakkelha-MIT).
- [Abjad CATT](https://github.com/abjadai/catt/tree/8d5330499feb85f6625e6af632141ed2ed6065fd): character/diacritic mappings in `data/catt.json` and corresponding reference fixtures. Apache-2.0; the full license is in [licenses/CATT-Apache-2.0](licenses/CATT-Apache-2.0). The mappings have been represented as JSON and paired with preservation-aware Rust tokenization; the upstream runtime is not bundled.

See the repository's [model validation](https://github.com/ieasybooks/tashkeel-rs/blob/main/docs/model-validation.md) for checkpoint provenance and known numerical differences. The native ONNX Runtime library is a separate MIT-licensed dependency; executable release archives retain its license and third-party notices alongside generated Rust dependency notices.
