# Releases

The repository follows the sibling projects: Conventional Commits, release-please, one shared workspace version, binary archives, and crates.io publication of the library before the CLI.

`release-please.yml` creates the version/changelog PR. After that PR merges, it creates a `vX.Y.Z` release and explicitly dispatches binary and crate publishing workflows because releases created with `GITHUB_TOKEN` do not trigger downstream release events. The bootstrap manifest starts at 0.0.0; the first feature release is 0.1.0.

Before using release-please, enable the repository setting that allows GitHub Actions to create pull requests. The workflow requests only its required repository permissions; organization policy can still restrict that setting.

The `release.yml` workflow bundles the pinned ONNX Runtime from `.github/scripts/onnxruntime.py`, generates dependency notices, and smoke-tests both models from an extracted archive in an unrelated working directory. Manual runs without `publish` keep CI artifacts only. Publishing requires a tag matching the root package version. The separate `models-v1` release does not trigger software publishing workflows.

`publish.yml` verifies the workspace packages, then publishes `tashkeel` before `tashkeel-cli`. First-time crates.io publication requires a repository `CARGO_REGISTRY_TOKEN` with appropriate access. After initial publication, configure trusted publishing for both crates with this repository, workflow `publish.yml`, and environment `release`, then remove the long-lived token. Environment protection rules are managed by the repository owner. Creating this repository or an implementation PR does not publish crates.

Model artifacts have an independent version. Preserve existing artifact URLs and hashes; publish changed weights under a new model revision, update the manifest and cache revision in `models.rs`, and rerun numerical parity, accuracy, and preservation tests before changing defaults. A model's quality status belongs in `docs/models.md` and the artifact release notes.
