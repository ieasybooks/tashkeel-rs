#!/usr/bin/env python3
"""Prove the CLI excludes CWD from Windows runtime dependency DLL searches.

Run on native Windows x64 with Python 3, the pinned Rust MSVC toolchain, a built
CLI, and the real Shakkelha model directory (the CLI verifies its pinned hash).
Every load uses a fresh process so the unrestricted control cannot preload a
dependency into a later test. This script does not download models or runtimes.
"""

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[2]
DEPENDENCY_SOURCE = """
#[unsafe(no_mangle)]
pub extern "C" fn tashkeel_fixture_dependency() -> u32 {
    73
}
"""
TOP_SOURCE = """
#[link(name = "DEPENDENCY_NAME", kind = "raw-dylib")]
unsafe extern "C" {
    fn tashkeel_fixture_dependency() -> u32;
}

// Exporting a call to the dependency keeps its import live in the PE image.
// Deliberately omit OrtGetApiBase: the CLI must reach symbol validation only
// when Windows can resolve the dependency from a trusted directory.
#[unsafe(no_mangle)]
pub extern "C" fn tashkeel_fixture_value() -> u32 {
    unsafe { tashkeel_fixture_dependency() }
}
"""
UNRESTRICTED_CONTROL = """
import ctypes
import sys

library = ctypes.WinDLL(sys.argv[1], winmode=0)
function = library.tashkeel_fixture_value
function.argtypes = []
function.restype = ctypes.c_uint32
if function() != 73:
    raise SystemExit("fixture dependency returned an unexpected value")
"""
RESTRICTED_ERROR = "with restricted Windows dependency search:"
MISSING_API_ERROR = "does not export `OrtGetApiBase`"


def run(command, cwd, env=None):
    return subprocess.run(
        [str(arg) for arg in command],
        cwd=cwd,
        env=env,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
        timeout=120,
        check=False,
    )


def require(condition, message, result):
    if not condition:
        raise SystemExit(
            f"{message}\nexit code: {result.returncode}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )


def compile_dll(source, output, crate_name):
    result = run(
        [
            "rustc", "--edition=2024", "--crate-type=cdylib",
            "--crate-name", crate_name, "-C", "panic=abort", source, "-o", output,
        ],
        # Keep rustup's toolchain selection tied to rust-toolchain.toml, even
        # though the fixture sources and outputs live outside the workspace.
        ROOT,
    )
    require(result.returncode == 0, "Could not compile the Windows DLL fixture", result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--model-dir", required=True, type=Path)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("native Windows is required; this regression must not be silently skipped")
    binary = args.binary.resolve(strict=True)
    model_dir = args.model_dir.resolve(strict=True)
    if not binary.is_file() or not (model_dir / "model.onnx").is_file():
        parser.error("supply a built CLI and the directory containing the pinned Shakkelha model.onnx")

    with tempfile.TemporaryDirectory(prefix="tashkeel-windows-loader-") as tmp:
        directory = Path(tmp).resolve()
        build = directory / "build"
        trusted = directory / "trusted-runtime"
        working = directory / "untrusted-cwd"
        for path in (build, trusted, working):
            path.mkdir()
        # A unique import name rules out satisfying the dependency from a
        # system directory, another installed package, or an existing module.
        dependency_name = "tashkeel_loader_dependency_" + uuid.uuid4().hex
        dependency_source = build / "dependency.rs"
        dependency_source.write_text(DEPENDENCY_SOURCE, encoding="utf-8")
        top_source = build / "top.rs"
        top_source.write_text(TOP_SOURCE.replace("DEPENDENCY_NAME", dependency_name), encoding="utf-8")
        dependency = build / f"{dependency_name}.dll"
        library = trusted / "onnxruntime.dll"
        compile_dll(dependency_source, dependency, "fixture_dependency")
        compile_dll(top_source, library, "fixture_top")
        dependency = dependency.replace(working / dependency.name)

        control = run([sys.executable, "-I", "-c", UNRESTRICTED_CONTROL, library], working)
        require(control.returncode == 0, "Unrestricted control must load the dependency from CWD", control)
        print("PASS: unrestricted WinDLL(winmode=0) loads the CWD-only dependency", flush=True)

        env = os.environ.copy()
        env.pop("ORT_DYLIB_PATH", None)
        command = [
            binary, "--model", "shakkelha", "--model-dir", model_dir,
            "--ort-library", library, "--offline", "--quiet", "--threads", "1",
            "--text", "اللغة العربية جميلة",
        ]
        rejected = run(command, working, env)
        require(
            rejected.returncode == 1
            and not rejected.stdout
            and RESTRICTED_ERROR in rejected.stderr
            and "OrtGetApiBase" not in rejected.stderr,
            "CLI must reject the CWD-only dependency before OrtGetApiBase validation",
            rejected,
        )
        print("PASS: CLI rejects the CWD-only dependency during restricted loading", flush=True)

        dependency.replace(trusted / dependency.name)
        adjacent = run(command, working, env)
        require(
            adjacent.returncode == 1
            and not adjacent.stdout
            and MISSING_API_ERROR in adjacent.stderr
            and RESTRICTED_ERROR not in adjacent.stderr,
            "CLI must allow the adjacent dependency and reach missing-OrtGetApiBase validation",
            adjacent,
        )
        print("PASS: CLI loads the adjacent dependency and reports missing OrtGetApiBase", flush=True)


if __name__ == "__main__":
    main()
