#!/usr/bin/env python3
"""Provision checksum-pinned CPU ONNX Runtime for CI and release bundles.

Python is build/setup tooling only; the Rust executable loads the native library.
"""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import tarfile
import urllib.request
import zipfile

VERSION = "1.31.0"
ARTIFACTS = {
    "x86_64-unknown-linux-gnu": ("linux-x64", "tgz", 11408003, "cc5c72baf5ae5c8238a6841f0897227be2d02826b9cf98eaf02fdefaeeb03a57"),
    "aarch64-unknown-linux-gnu": ("linux-aarch64", "tgz", 10331104, "c5b8b3cca31f3d643a3b313b8f42e2d0f16a4fcefd02cda343d97bd5306afaab"),
    "aarch64-apple-darwin": ("osx-arm64", "tgz", 36549009, "1031b36dd3bdaa8d976a6130d9cf2a05e9be320b93ae804f8427b91dc3432afc"),
    "x86_64-pc-windows-msvc": ("win-x64", "zip", 82887349, "3958d8a44984160692c1b4ffd0de3cb48f8c39f9d34bb31e433db06e30b82195"),
}


def host_target():
    system = platform.system()
    arm = platform.machine().lower() in ("arm64", "aarch64")
    if system == "Linux":
        return "aarch64-unknown-linux-gnu" if arm else "x86_64-unknown-linux-gnu"
    if system == "Darwin" and arm:
        return "aarch64-apple-darwin"
    if system == "Windows" and not arm:
        return "x86_64-pc-windows-msvc"
    raise SystemExit("No pinned native runtime for this host; supply --target or provision ONNX Runtime 1.31 yourself")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", choices=ARTIFACTS, default=os.environ.get("TARGET"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    target = args.target or host_target()
    name, extension, size, digest = ARTIFACTS[target]
    root = f"onnxruntime-{name}-{VERSION}"
    url = f"https://github.com/microsoft/onnxruntime/releases/download/v{VERSION}/{root}.{extension}"
    output = (args.output or Path("target") / "onnxruntime" / target).resolve()
    output.mkdir(parents=True, exist_ok=True)
    archive_path = output / f"runtime.{extension}"
    if archive_path.is_file():
        data = archive_path.read_bytes()
    else:
        request = urllib.request.Request(url, headers={"User-Agent": "tashkeel-rs runtime setup"})
        with urllib.request.urlopen(request, timeout=180) as response:
            data = response.read(size + 1)
    if len(data) != size or hashlib.sha256(data).hexdigest() != digest:
        raise SystemExit(f"ONNX Runtime archive size or SHA-256 mismatch: {archive_path}")
    archive_path.write_bytes(data)
    if name.startswith("linux"):
        library, installed = f"lib/libonnxruntime.so.{VERSION}", "libonnxruntime.so"
    elif name.startswith("osx"):
        library, installed = f"lib/libonnxruntime.{VERSION}.dylib", "libonnxruntime.dylib"
    else:
        library, installed = "lib/onnxruntime.dll", "onnxruntime.dll"
    selected = {library: installed, "LICENSE": "ONNXRUNTIME-LICENSE", "ThirdPartyNotices.txt": "ONNXRUNTIME-NOTICES"}
    # Read only specific files; archive paths never become output paths.
    if extension == "zip":
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            for source, destination in selected.items():
                (output / destination).write_bytes(archive.read(f"{root}/{source}"))
    else:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            for source, destination in selected.items():
                stream = archive.extractfile(f"{root}/{source}")
                if stream is None:
                    raise SystemExit(f"Missing runtime file: {source}")
                (output / destination).write_bytes(stream.read())
    runtime = output / installed
    (output / "manifest.json").write_text(json.dumps({"version": VERSION, "target": target, "url": url, "sha256": digest}, indent=2) + "\n")
    if os.environ.get("GITHUB_ENV"):
        with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as env:
            env.write(f"ORT_DYLIB_PATH={runtime}\nTASHKEEL_RUNTIME_DIR={output}\n")
    print(runtime)


if __name__ == "__main__":
    main()
