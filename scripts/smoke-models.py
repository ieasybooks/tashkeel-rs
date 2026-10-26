#!/usr/bin/env python3
"""Check real CPU inference, source preservation, and offline cached execution."""

import argparse
import os
from pathlib import Path
import subprocess
import tempfile

MARKS = set("\u064b\u064c\u064d\u064e\u064f\u0650\u0651\u0652")
TEXT = "  اللغة العربية جميلة، وفي عام 2026 نكتب Rust.\nهذا اختبار للتشكيل.\t\n"


def bare(text):
    return "".join(c for c in text if c not in MARKS)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix="tashkeel-smoke-") as tmp:
        cache = Path(os.environ.get("TASHKEEL_SMOKE_CACHE", Path(tmp) / "cache"))
        for model in ("shakkelha", "catt"):
            command = [str(binary), "--model", model, "--text", TEXT, "--cache-dir", str(cache), "--quiet"]
            result = subprocess.run(command, cwd=tmp, capture_output=True, check=True, encoding="utf-8")
            assert bare(result.stdout) == bare(TEXT), (model, "source text changed", result.stdout)
            assert any(c in MARKS for c in result.stdout), (model, "no diacritics predicted")
            offline = subprocess.run(command + ["--offline"], cwd=tmp, capture_output=True, check=True, encoding="utf-8")
            assert result.stdout == offline.stdout, (model, "cached inference differs")
            print(f"{model}: CPU inference, preserved source, deterministic offline cache: OK")


if __name__ == "__main__":
    main()
