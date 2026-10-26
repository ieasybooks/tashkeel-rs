# Native runtime

Serving requires the Rust application, CPU ONNX Runtime **1.31.0**, and the selected model files. Python, TensorFlow, and PyTorch are not serving dependencies. The Rust `ort` binding is pinned to **2.0.0-rc.15** with API 31 and dynamic loading; it has no stable 2.0 release at the time of this implementation.

The library resolves the runtime in this order: `InferenceOptions::runtime_path` (CLI `--ort-library`), `ORT_DYLIB_PATH`, a library next to the executable, then the Unix platform loader's library name. Explicit paths must exist and take precedence; a configured path is not silently replaced. Windows requires an explicit or adjacent library because its default DLL search can include the working directory. A process cannot switch to another ONNX Runtime binary after initialization.

On Windows, the selected canonical absolute library path is first loaded with `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`. Dependency searches use the selected library's directory and Windows System32; a missing Visual C++ runtime dependency cannot fall back to an unrelated DLL in the working directory or on `PATH`. The preload handle stays alive through `ort::init_from(...).build()`, after which the pinned binding retains its own reference to the already-loaded module. This changes neither process-wide DLL search settings nor the working directory. Runtime selection is a trust decision: the selected library and its directory must be trusted, and this policy does not isolate already-loaded modules or code executed by native libraries.

The pinned `ort` API exposes no loading-flags option and internally uses `LoadLibraryExW` with flags zero. The private Windows preload therefore contains one audited `unsafe` call to `libloading` 0.9, with a local `#[allow(unsafe_code)]` and its safety rationale. Workspace code otherwise uses `unsafe_code = "deny"`; `forbid` would prevent this narrowly scoped exception. `libloading` was already a transitive dependency of `ort`, and remains covered by the generated Rust dependency notices.

## Windows dependency-search regression

On a native Windows x64 runner with Python 3, the pinned Rust MSVC toolchain, a built CLI, and the pinned Shakkelha model directory, run:

```powershell
python .github/scripts/windows-loader-regression.py --binary target/debug/tashkeel.exe --model-dir C:/path/to/shakkelha/v1
```

The script compiles two small Rust DLL fixtures outside the workspace. A fixture `onnxruntime.dll` imports a uniquely named dependency initially present only in the working directory. A fresh Python process must load it with unrestricted `ctypes.WinDLL(..., winmode=0)`, confirming that the fixture exercises the vulnerable search. A separate CLI process must fail during restricted dependency loading, before validating `OrtGetApiBase`. Moving the dependency beside the selected runtime must instead reach the expected missing-`OrtGetApiBase` error. The CLI verifies the real model files before both attempts; no model or runtime downloads occur in this test. Native Windows proof remains unverified until all three checks pass in CI; Linux tests or cross-compilation do not establish this behavior. Keep the normal Windows smoke checks for both real models as well.

## Source installation

Run `python3 .github/scripts/onnxruntime.py` from a checkout. It downloads the official Microsoft release, verifies its pinned archive SHA-256, and prints the shared library's absolute path. Set `ORT_DYLIB_PATH` to that path or pass it with `--ort-library`.

The helper supports Linux x64 and ARM64, macOS ARM64, and Windows x64. It can also provision another target using `--target`, and accepts `--output DIR`. On Windows use `python` instead of `python3` if that is the installed launcher.

Alternatively, download the matching **CPU** archive from the [official v1.31.0 release](https://github.com/microsoft/onnxruntime/releases/tag/v1.31.0), extract its shared library, and pass the path explicitly. Linux uses `libonnxruntime.so`, macOS uses `libonnxruntime.dylib`, and Windows uses `onnxruntime.dll`. The helper renames the versioned Unix library to the unversioned name without changing its contents.

## Release archives

Archives contain the executable, native library, CLI reference/man page, MIT license, model notices, Rust dependency notices, and ONNX Runtime's third-party notices. Keep the native library next to the executable when moving an installation. Model weights download separately so the two backends can be provisioned independently.

Windows x64 also requires the [Microsoft Visual C++ v14 x64 Redistributable](https://aka.ms/vc14/vc_redist.x64.exe), available from [Microsoft's supported downloads](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist). Install it before running inference if it is not already present. The pinned ONNX Runtime DLL imports `MSVCP140.dll`, `MSVCP140_1.dll`, `VCRUNTIME140.dll`, and `VCRUNTIME140_1.dll`; the Rust executable's static C runtime does not supply these dependencies. The redistributable is a prerequisite, not bundled or installed automatically. Hosted Windows CI already has it, so that CI does not establish operation on a clean Windows installation without the prerequisite.

The release matrix deliberately contains only platforms with a pinned upstream native runtime. There is no upstream macOS Intel or Linux musl v1.31.0 archive in the inspected release. Those platforms require separately building/provisioning a compatible runtime and testing the resulting application; they are not advertised as prebuilt targets here.
