# AGENTS.md

Read `README.md` and use the toolchain pinned in `rust-toolchain.toml`. Follow the sibling tafrigh-rs and tahweel-rs workspace conventions: the root package is `tashkeel-cli`, the binary is `tashkeel`, and reusable code lives in `crates/tashkeel`.

Use workspace dependencies and lint settings, Conventional Commits, and concise documentation with detailed explanations in `docs/`. Keep each prose paragraph on one source line. CLI declarations live in `src/cli.rs`; regenerate CLI reference and man page after changing them.

Preserve original base characters, punctuation, whitespace, and document structure. Model normalization must not become output normalization. Keep inference synchronous and bounded so callers can run it on a blocking worker. Use pinned model artifacts and verify hashes. Never commit model weights or credentials.

Run focused tests while iterating. Before handing off implementation changes, run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --locked`. Model-free tests require no downloads. Run the opt-in real-model tests when model files and ONNX Runtime are available, and state any unverified compatibility or accuracy claims.
