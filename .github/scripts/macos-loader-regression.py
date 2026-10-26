#!/usr/bin/env python3
"""Prove the CLI refuses a CWD-only macOS runtime without executing its code.

Run on native macOS with Python 3, Apple clang, a built CLI, and the pinned
Shakkelha model directory (the CLI verifies its hash). A native dlopen control
first proves the fixture's constructor runs. Every load uses a fresh process.
This script does not download models or runtimes.
"""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

LIBRARY_SOURCE = r"""
#include <stdio.h>
#include <stdlib.h>

__attribute__((constructor))
static void mark_loaded(void) {
    const char *path = getenv("TASHKEEL_LOADER_MARKER");
    if (path == NULL) abort();
    FILE *file = fopen(path, "w");
    if (file == NULL) abort();
    if (fputs("constructor executed\n", file) < 0 || fclose(file) != 0) abort();
}
"""
CONTROL_SOURCE = r"""
#include <dlfcn.h>
#include <stdio.h>

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    void *library = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (library == NULL) {
        fprintf(stderr, "%s\n", dlerror());
        return 1;
    }
    return dlclose(library) != 0;
}
"""
REQUIRED_PATH_ERROR = "macOS requires libonnxruntime.dylib beside the executable"
MISSING_API_ERROR = "does not export `OrtGetApiBase`"
MARKER_CONTENT = "constructor executed\n"


def run(command, cwd, env):
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


def marked(marker):
    return marker.is_file() and marker.read_text(encoding="utf-8") == MARKER_CONTENT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--model-dir", required=True, type=Path)
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("native macOS is required; this regression must not be silently skipped")
    binary = args.binary.resolve(strict=True)
    model_dir = args.model_dir.resolve(strict=True)
    if not binary.is_file() or not (model_dir / "model.onnx").is_file():
        parser.error("supply a built CLI and the directory containing the pinned Shakkelha model.onnx")
    compiler = shutil.which("clang")
    if compiler is None:
        parser.error("Apple clang is required to compile the native constructor fixture")

    with tempfile.TemporaryDirectory(prefix="tashkeel-macos-loader-") as tmp:
        directory = Path(tmp).resolve()
        build = directory / "build"
        installed = directory / "installed"
        working = directory / "untrusted-cwd"
        for path in (build, installed, working):
            path.mkdir()
        executable = installed / "tashkeel"
        shutil.copy2(binary, executable)
        marker = directory / "constructor-marker"
        # Remove runtime overrides for every child, including DYLD injection
        # and search settings. The CLI's only nearby dylib will be in CWD.
        env = {
            key: value for key, value in os.environ.items()
            if key != "ORT_DYLIB_PATH" and not key.startswith("DYLD_")
        }
        env["TASHKEEL_LOADER_MARKER"] = str(marker)

        library = working / "libonnxruntime.dylib"
        source = build / "fixture.c"
        source.write_text(LIBRARY_SOURCE, encoding="utf-8")
        compiled = run(
            [compiler, "-dynamiclib", "-Wall", "-Wextra", "-Werror", source, "-o", library],
            build,
            env,
        )
        require(compiled.returncode == 0, "Could not compile the dylib fixture", compiled)
        source = build / "control.c"
        source.write_text(CONTROL_SOURCE, encoding="utf-8")
        control_binary = build / "control"
        compiled = run(
            [compiler, "-Wall", "-Wextra", "-Werror", source, "-o", control_binary],
            build,
            env,
        )
        require(compiled.returncode == 0, "Could not compile the dlopen control", compiled)

        # An exact-path control is independent of dyld search policy changes.
        # It proves the native fixture is loadable and its constructor really
        # writes the marker; a missing/broken fixture cannot yield a false pass.
        control = run([control_binary, library], working, env)
        require(
            control.returncode == 0 and marked(marker),
            "Native dlopen control must execute the fixture constructor",
            control,
        )
        marker.unlink()
        print("PASS: native dlopen executes the fixture constructor", flush=True)

        command = [
            executable, "--model", "shakkelha", "--model-dir", model_dir,
            "--offline", "--text", "لغة عربية",
        ]
        rejected = run(command, working, env)
        require(
            rejected.returncode == 1
            and not rejected.stdout
            and REQUIRED_PATH_ERROR in rejected.stderr
            and "OrtGetApiBase" not in rejected.stderr
            and not marker.exists(),
            "CLI must require an explicit/adjacent runtime without executing the CWD dylib",
            rejected,
        )
        print("PASS: CLI rejects the CWD-only runtime before executing its constructor", flush=True)

        # Keep positive coverage of both allowed choices. The harmless fixture
        # deliberately lacks the real API, so symbol validation must fail only
        # after its constructor runs. Each attempt still verifies the model.
        explicit = run([*command, "--ort-library", library], working, env)
        require(
            explicit.returncode == 1
            and not explicit.stdout
            and MISSING_API_ERROR in explicit.stderr
            and marked(marker),
            "An explicitly selected fixture must load and reach OrtGetApiBase validation",
            explicit,
        )
        marker.unlink()
        print("PASS: an explicit runtime path reaches symbol validation", flush=True)

        library.replace(installed / library.name)
        adjacent = run(command, working, env)
        require(
            adjacent.returncode == 1
            and not adjacent.stdout
            and MISSING_API_ERROR in adjacent.stderr
            and marked(marker),
            "An adjacent fixture must load and reach OrtGetApiBase validation",
            adjacent,
        )
        print("PASS: an adjacent runtime reaches symbol validation", flush=True)


if __name__ == "__main__":
    main()
