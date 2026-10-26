# Dependencies and notices

The application and library are MIT licensed. `docxml` is dual MIT/Apache-2.0; ONNX Runtime is MIT. Model licenses and attribution remain separate from the application's license; see [models](models.md) and `licenses/`. The published library also includes [tokenizer/fixture notices](../crates/tashkeel/THIRD-PARTY-NOTICES.md) and both upstream license texts.

The release workflow generates complete Rust dependency notices with `cargo about` and includes the pinned ONNX Runtime archive's license and third-party notices. The compression dependencies brought in by `docxml` include `libbz2-rs-sys` (bzip2-1.0.6) and `ppmd-rust` (MIT-0 selected from its dual license). These permit redistribution; retain their notices as generated. Other allowed licenses are checked by `cargo deny`.

## Scoped ZIP advisory exceptions

`docxml` is pinned to 0.3.1 and depends on `zip` 7.4.0. Three RustSec advisories currently flag that transitive dependency:

- [RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367): preservation of privileged Unix permission bits during extraction.
- [RUSTSEC-2026-0368](https://rustsec.org/advisories/RUSTSEC-2026-0368): filesystem symlink targets outside an extraction directory.
- [RUSTSEC-2026-0369](https://rustsec.org/advisories/RUSTSEC-2026-0369): applying permissions through extracted filesystem symlinks.

All three concern filesystem extraction APIs. Tashkeel does not call `ZipArchive::extract`, `ZipStreamReader::extract`, create filesystem symlinks from archive entries, or apply archived Unix permissions. `docxml` 0.3.1's OPC reader uses `by_index` and in-memory reads; its writer emits ZIP bytes without filesystem extraction. Our input/output handling and model downloads likewise do not invoke the affected APIs. Model ZIP entries are pinned by whole-archive and individual-file digests and written only to predetermined filenames.

The three IDs are explicit exceptions in `deny.toml` for these unreachable operations. Other advisories still fail the dependency check. Review the call paths and remove these exceptions when `docxml` upgrades to a corrected ZIP dependency; do not add generic advisory exemptions or switch to archive extraction without revisiting this assessment.
