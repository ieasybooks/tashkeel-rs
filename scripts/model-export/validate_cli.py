#!/usr/bin/env python3
"""Score the built Rust CLI's default Shakkelha pipeline on pinned Fadel gold.

Uses only local files and the Python standard library; no builds or downloads.
The entire UTF-8 document is submitted once, without stripping line whitespace
or overriding the CLI's 500-character / one-thread defaults. This measures this
corpus and CPU pipeline, not general quality or live historical GPU equivalence.
"""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[2]
GOLD_SHA256 = "4e851ff836f0a178abb15d9f4a8bcf92748b77cc0d67030d9fae2fbe698baa12"
EVALUATOR_SHA256 = "535b729c349965bfcb6863fc65c06a5465d3b900128ce19ce135998620e32fc8"
MODEL_SHA256 = "41ed03075e80847aaef2bfdb65f123b7f225ff6a179a507b1ec3e6152184fa71"
VOCABULARY_SHA256 = "e81c0a089cbfd95eaef3d7130c2c3e1f0cc5f7471f9c4e71f807b7ebba1eccb1"
MARKS = frozenset(chr(code) for code in range(0x064B, 0x0653))


def sha256(path):
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def strip_marks(text):
    return "".join(char for char in text if char not in MARKS)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["binary", "model-dir", "ort-library", "gold", "evaluator", "report"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/validation/cli")
    parser.add_argument("--baseline", type=Path, default=Path(__file__).with_name("validation-full.json"))
    parser.add_argument("--timeout-seconds", type=int, default=300, help="CLI inference wall-clock limit")
    args = parser.parse_args()
    if args.timeout_seconds <= 0:
        parser.error("--timeout-seconds must be positive")
    for name, value in vars(args).items():
        if isinstance(value, Path):
            setattr(args, name, value.resolve())
    if not any(args.output_dir.is_relative_to(root.resolve()) for root in [ROOT / "target/validation", Path("/tmp")]):
        parser.error("large text outputs must be under target/validation or /tmp")

    vocabulary_path = ROOT / "crates/tashkeel/data/shakkelha.json"
    model_path = args.model_dir / "model.onnx"
    for path, expected in [
        (args.gold, GOLD_SHA256), (args.evaluator, EVALUATOR_SHA256),
        (model_path, MODEL_SHA256), (vocabulary_path, VOCABULARY_SHA256),
    ]:
        if sha256(path) != expected:
            raise ValueError(f"pinned SHA-256 mismatch: {path}")
    vocabulary = json.loads(vocabulary_path.read_text(encoding="utf-8"))
    if set(vocabulary["diacritics"]) != MARKS:
        raise ValueError("vocabulary does not use the expected eight marks")
    baseline = json.loads(args.baseline.read_text(encoding="utf-8"))
    if (baseline["gold_sha256"] != GOLD_SHA256
            or baseline["model_sha256"] != MODEL_SHA256
            or baseline["fadel_evaluator"]["sha256"] != EVALUATOR_SHA256
            or baseline["saved_output_comparison"]["lines"] != 2500):
        raise ValueError("whole-line baseline does not match the pinned evaluation")
    gold = args.gold.read_bytes().decode("utf-8")
    if len(gold.splitlines()) != 2500:
        raise ValueError("expected exactly 2500 gold lines")
    source = strip_marks(gold)
    source_bytes = source.encode("utf-8")
    help_text = subprocess.check_output([str(args.binary), "--help"], encoding="utf-8", timeout=15)
    for flag, value in [("threads", 1), ("chunk-size", 500)]:
        if not re.search(rf"--{flag}\b(?:(?!--).)*?\(default: {value}\)", help_text, re.S):
            raise ValueError(f"binary does not advertise the expected --{flag} default")

    input_path = args.output_dir / "input.txt"
    output_path = args.output_dir / "predicted.txt"
    stderr_path = args.output_dir / "stderr.txt"
    inputs = {path.resolve() for path in [args.binary, args.gold, args.evaluator, args.baseline,
                                         args.ort_library, model_path, vocabulary_path, Path(__file__)]}
    outputs = [path.resolve() for path in [input_path, output_path, stderr_path, args.report]]
    if len(set(outputs)) != len(outputs) or inputs.intersection(outputs):
        parser.error("output paths must be distinct and must not overwrite inputs")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    input_path.write_bytes(source_bytes)
    command = [str(args.binary), "-", "--model", "shakkelha", "--model-dir", str(args.model_dir),
               "--offline", "--quiet"]
    environment = os.environ.copy()
    environment["ORT_DYLIB_PATH"] = str(args.ort_library)
    binary_hash = sha256(args.binary)
    runtime_hash = sha256(args.ort_library)
    print(f"Running default CLI on 2500 lines / {len(source)} source characters", flush=True)
    started = time.monotonic()
    with input_path.open("rb") as stdin, output_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        result = subprocess.run(command, stdin=stdin, stdout=stdout, stderr=stderr,
                                env=environment, timeout=args.timeout_seconds, check=False)
    elapsed = time.monotonic() - started
    if result.returncode:
        raise RuntimeError(f"CLI exit {result.returncode}: {stderr_path.read_text(encoding='utf-8')[:4000]}")
    if sha256(args.binary) != binary_hash:
        raise ValueError("binary changed during inference; rerun with a stable executable")
    predicted_bytes = output_path.read_bytes()
    predicted = predicted_bytes.decode("utf-8")
    if strip_marks(predicted).encode("utf-8") != source_bytes:
        raise ValueError("CLI changed base text, punctuation, whitespace, or document structure")
    if len(predicted.splitlines()) != 2500:
        raise ValueError("CLI did not preserve the 2500 corpus lines")
    print(f"Inference finished in {elapsed:.1f}s; exact source preserved; scoring four settings", flush=True)

    sys.dont_write_bytecode = True
    spec = importlib.util.spec_from_file_location("fadel_evaluator", args.evaluator)
    evaluator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(evaluator)
    classes = [vocabulary["classes"][str(index)] for index in range(1, 15)]
    scores = []
    comparisons = []
    for case_endings, include_no_diacritic in [(True, True), (False, True), (True, False), (False, False)]:
        settings = {"case_endings": case_endings, "include_no_diacritic": include_no_diacritic}
        kwargs = {"case_ending": case_endings, "no_diacritic": include_no_diacritic}
        score = dict(settings)
        for metric, calculate in [("DER_percent", evaluator.calculate_der), ("WER_percent", evaluator.calculate_wer)]:
            score[metric] = calculate(args.gold, output_path, vocabulary["arabic_letters"], classes, "Fadel", **kwargs)
        scores.append(score)
        comparison = dict(settings)
        for name in ["onnx", "published"]:
            reference = next(item for item in baseline["fadel_evaluator"]["results"][name]
                             if all(item[key] == value for key, value in settings.items()))
            comparison[name] = {
                metric: reference[metric] for metric in ["DER_percent", "WER_percent"]
            }
            comparison[name].update({
                metric.replace("_percent", "_delta_percentage_points"): round(score[metric] - reference[metric], 2)
                for metric in ["DER_percent", "WER_percent"]
            })
        comparisons.append(comparison)
        print(json.dumps(score), flush=True)

    report = {
        "scope": "Fixed 2500-line Fadel gold corpus through the actual Rust CLI's default Shakkelha CPU document pipeline; not general quality or live historical GPU equivalence",
        "gold_sha256": GOLD_SHA256, "model_sha256": MODEL_SHA256,
        "vocabulary_sha256": VOCABULARY_SHA256,
        "binary": {"path": str(args.binary), "sha256": binary_hash},
        "runtime": {"path": str(args.ort_library), "sha256": runtime_hash},
        "command": command,
        "environment": {"ORT_DYLIB_PATH": str(args.ort_library)},
        "defaults": {"chunk_size": 500, "threads": 1, "verified_from_binary_help": True, "flags_omitted": True},
        "input": {"gold_path": str(args.gold), "lines": 2500, "source_characters": len(source),
                  "source_bytes": len(source_bytes), "source_sha256": sha256(input_path),
                  "removed_marks": [f"U+{ord(char):04X}" for char in sorted(MARKS)],
                  "submission": "one complete UTF-8 document on stdin; no line stripping or newline normalization"},
        "output": {"path": str(output_path), "sha256": sha256(output_path), "lines": len(predicted.splitlines()),
                   "bytes": len(predicted_bytes), "exact_source_preserved_after_stripping_marks": True,
                   "stderr_path": str(stderr_path), "stderr_bytes": stderr_path.stat().st_size},
        "inference_wall_seconds": round(elapsed, 3),
        "timing_note": "Single shared-host run; not a benchmark",
        "fadel_evaluator": {"path": str(args.evaluator), "sha256": EVALUATOR_SHA256,
                            "style": "Fadel", "results": {"rust_cli_default": scores}},
        "whole_line_comparison": {"baseline_path": str(args.baseline), "baseline_sha256": sha256(args.baseline),
                                  "scope": baseline["scope"], "results": comparisons,
                                  "delta_note": "CLI minus existing whole-line scores, in percentage points; upstream scores round to two decimals"},
    }
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Report: {args.report}", flush=True)


if __name__ == "__main__":
    main()
