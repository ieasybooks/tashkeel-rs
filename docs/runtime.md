# Native runtime

Serving requires the Rust application, CPU ONNX Runtime **1.31.0**, and the selected model files. Python, TensorFlow, and PyTorch are not serving dependencies. The Rust `ort` binding is pinned to **2.0.0-rc.15** with API 31 and dynamic loading; it has no stable 2.0 release at the time of this implementation.

`Diacritizer::load` resolves the runtime in this order: `InferenceOptions::runtime_path` (CLI `--ort-library`), `ORT_DYLIB_PATH`, then a library next to the executable. Selected paths are canonicalized before loading; explicit paths must exist and take precedence, and a configured path is not silently replaced. Windows and macOS require an explicit or adjacent library because bare library-name loading can search the working directory. Other platforms, including Linux, retain the platform loader's library-name fallback. A process cannot switch to another ONNX Runtime binary after initialization.

On macOS, passing a bare `libonnxruntime.dylib` to `dlopen` does not exclude the working directory; Apple's [dynamic library usage guide](https://developer.apple.com/library/archive/documentation/DeveloperTools/Conceptual/DynamicLibraries/100-Articles/DynamicLibraryUsageGuidelines.html) documents that search. The loader therefore fails before calling `ort` when no explicit or adjacent path is available. It changes neither the working directory nor process-wide loader settings. As on other platforms, selecting a native library is a trust decision; its dependencies and initialization code remain native code executing in the application.

On Windows, the selected canonical absolute library path is first loaded with `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`. Dependency searches use the selected library's directory and Windows System32; a missing Visual C++ runtime dependency cannot fall back to an unrelated DLL in the working directory or on `PATH`. The preload handle stays alive through `ort::init_from(...).build()`, after which the pinned binding retains its own reference to the already-loaded module. This changes neither process-wide DLL search settings nor the working directory. Runtime selection is a trust decision: the selected library and its directory must be trusted, and this policy does not isolate already-loaded modules or code executed by native libraries.

The pinned `ort` API exposes no loading-flags option and internally uses `LoadLibraryExW` with flags zero. The private Windows preload therefore contains one audited `unsafe` call to `libloading` 0.9, with a local `#[allow(unsafe_code)]` and its safety rationale. Workspace code otherwise uses `unsafe_code = "deny"`; `forbid` would prevent this narrowly scoped exception. `libloading` was already a transitive dependency of `ort`, and remains covered by the generated Rust dependency notices.

## Host-owned environments

The pinned `ort` binding permits only one environment creation per process, even after all handles have been dropped. Standalone `Diacritizer::load` retains its environment for reuse by subsequent engines. Applications that already initialize `ort` must instead pass that environment to `Diacritizer::load_with_environment`; see the [integration example](integration.md#host-owned-ort-environment). This method does not initialize or discover a runtime, apply the standalone Windows preload, or read `ORT_DYLIB_PATH`. `InferenceOptions::runtime_path` must be `None`, even if it names the host's runtime; specifying it is an error because the host has already selected the binary. Native runtime and dependency loading are the host's responsibility.

Both loading methods build CPU sessions with automatic device selection disabled, without inheriting the host's default execution providers. Each session uses an independent thread pool: `InferenceOptions::threads` controls intra-operation threads, inter-operation threads are one, graph execution is sequential, and spinning is disabled. Host global pools do not override these settings; the host's own sessions retain their configuration. Sessions hold shared ownership of the environment, so an engine remains valid after the caller drops its environment handle. Retain a host handle if more engines will be created later.

## macOS working-directory regression

On a native macOS runner with Python 3, Apple clang, a built CLI, and the pinned Shakkelha model directory, run:

```sh
python3 .github/scripts/macos-loader-regression.py --binary target/debug/tashkeel --model-dir /path/to/shakkelha/v1
```

The script copies the CLI into an isolated temporary directory without an adjacent runtime and clears `ORT_DYLIB_PATH` and `DYLD_*` overrides in child processes. It compiles a harmless `libonnxruntime.dylib` with a constructor that writes a marker, placing it in a separate working directory. A native `dlopen` control loads the fixture by its exact path and must produce the marker, proving the fixture actually executes. A fresh CLI process must report the explicit-or-adjacent path requirement without writing the marker. Further fresh CLI processes explicitly select the fixture, then find it adjacent to the executable; both must write the marker and reach the expected missing-`OrtGetApiBase` error. Each CLI attempt verifies the real model before runtime selection, and every subprocess has a 120-second timeout. No downloads occur. Native macOS proof remains unverified until the script passes on macOS; Linux tests and cross-compilation do not establish loader behavior.

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
