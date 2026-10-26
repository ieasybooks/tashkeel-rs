#!/usr/bin/env python3
"""Check numerical conversion and saved-output fidelity; optionally run Fadel's evaluator.

Inputs are explicit local files. No downloads, timing benchmarks, GPU inference,
or serving are performed. Corpus inference uses one unchunked line per call, as
upstream predict.py did; it does not measure the Rust library's default chunks.
"""

import os
for variable in ["OMP_NUM_THREADS", "OPENBLAS_NUM_THREADS", "MKL_NUM_THREADS"]:
    os.environ[variable] = "1"

import argparse
import importlib.util
import json
from pathlib import Path

import numpy as np
import onnxruntime as ort

from export_shakkelha import MODEL_SHA256, load_weights, reference, sha256

VOCABULARY = Path(__file__).resolve().parents[2] / "crates/tashkeel/data/shakkelha.json"
EVALUATOR_SHA256 = "535b729c349965bfcb6863fc65c06a5465d3b900128ce19ce135998620e32fc8"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument("--gold", type=Path, required=True)
    parser.add_argument("--published", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="predicted text; input files are never overwritten")
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--evaluator", type=Path, help="pinned upstream diacritization_stat.py")
    parser.add_argument("--limit", type=int, default=0, help="first N lines, or zero for full corpus")
    args = parser.parse_args()
    if args.limit < 0:
        parser.error("--limit cannot be negative")
    if any(args.output.resolve() == path.resolve() for path in [args.gold, args.published, args.model, args.checkpoint]):
        parser.error("--output must differ from input files")
    if sha256(args.model) != MODEL_SHA256:
        raise ValueError("model hash does not match the pinned sigmoid export")
    vocabulary = json.loads(VOCABULARY.read_text(encoding="utf-8"))
    marks = set(vocabulary["diacritics"])
    arabic = vocabulary["arabic_letters"]

    def strip(text):
        return "".join(c for c in text if c not in marks)

    def encode(text):
        return np.asarray([[1] + [vocabulary["characters"].get(c, 3) for c in strip(text)] + [2]], dtype=np.int64)

    def decode(text, probabilities):
        output = []
        for char, label in zip(strip(text), probabilities[0].argmax(-1)[1:-1], strict=True):
            diacritics = vocabulary["classes"][str(label)]
            output.append(char)
            if char in arabic and "<" not in diacritics:
                output.append(diacritics)
        return "".join(output)

    def labels(text):
        output = []
        for char in text:
            if char in marks and output:
                output[-1][1] += char
            else:
                output.append([char, ""])
        return output

    options = ort.SessionOptions()
    options.intra_op_num_threads = 1
    options.inter_op_num_threads = 1
    options.execution_mode = ort.ExecutionMode.ORT_SEQUENTIAL
    options.add_session_config_entry("session.intra_op.allow_spinning", "0")
    session = ort.InferenceSession(str(args.model), options, providers=["CPUExecutionProvider"])
    weights = load_weights(args.checkpoint)
    report = {
        "model_sha256": sha256(args.model), "gold_sha256": sha256(args.gold),
        "published_sha256": sha256(args.published), "onnxruntime": ort.__version__,
        "numpy": np.__version__, "numpy_recurrence": [],
        "scope": "unchunked line-level batch-one CPU model; not Rust default chunking or live historical GPU execution",
    }
    sample = "اللغة العربية لغة جميلة وغنية، وهي وسيلة للتواصل بين الناس ونقل العلوم والمعارف. يقرأ الطالب كتابا مفيدا في المكتبة ثم يعود إلى بيته. "
    for size in [30, 100, 500, 1000]:
        text = (sample * (size // len(sample) + 1))[:size]
        ids = encode(text)
        expected = reference(ids, weights)
        actual = session.run(None, {"input_ids": ids})[0]
        difference = float(np.abs(expected - actual).max())
        equal = bool(np.array_equal(expected.argmax(-1), actual.argmax(-1)))
        if difference >= 1e-4 or not equal:
            raise ValueError(f"NumPy recurrence mismatch at {size} characters: {difference}")
        report["numpy_recurrence"].append({"characters": size, "max_abs_probability_error": difference, "argmax_equal": equal})
    gold = [line.strip() for line in args.gold.read_text(encoding="utf-8").splitlines()]
    published = [line.strip() for line in args.published.read_text(encoding="utf-8").splitlines()]
    if len(gold) != len(published):
        raise ValueError("gold and published line counts differ")
    if args.limit:
        gold, published = gold[:args.limit], published[:args.limit]
    predicted = []
    total = differences = exact = 0
    for index, (source, saved) in enumerate(zip(gold, published, strict=True)):
        if strip(source) != strip(saved):
            raise ValueError(f"gold/published base text mismatch on line {index + 1}")
        output = decode(source, session.run(None, {"input_ids": encode(source)})[0])
        predicted.append(output)
        expected_labels, actual_labels = labels(saved), labels(output)
        if len(expected_labels) != len(actual_labels):
            raise ValueError(f"label alignment mismatch on line {index + 1}")
        total += len(expected_labels)
        differences += sum(a != b for a, b in zip(expected_labels, actual_labels, strict=True))
        exact += output == saved
        if (index + 1) % 250 == 0:
            print(f"validated {index + 1}/{len(gold)} lines", flush=True)
    args.output.write_text("\n".join(predicted) + "\n", encoding="utf-8")
    report["saved_output_comparison"] = {"lines": len(gold), "exact_lines": exact, "base_positions": total, "different_labels": differences}
    if args.evaluator:
        if sha256(args.evaluator) != EVALUATOR_SHA256:
            raise ValueError("evaluator differs from pinned upstream source")
        spec = importlib.util.spec_from_file_location("fadel_evaluator", args.evaluator)
        evaluator = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(evaluator)
        # The original functions accept filenames. Isolate stripped inputs used
        # above so line limits and final-newline handling match exactly.
        from tempfile import TemporaryDirectory
        with TemporaryDirectory() as directory:
            paths = {}
            for name, lines in [("gold", gold), ("published", published), ("onnx", predicted)]:
                paths[name] = Path(directory) / (name + ".txt")
                paths[name].write_text("\n".join(lines) + "\n", encoding="utf-8")
            classes = [vocabulary["classes"][str(i)] for i in range(1, 15)]
            report["fadel_evaluator"] = {"sha256": EVALUATOR_SHA256, "results": {}}
            for name in ["published", "onnx"]:
                results = []
                for case_ending, no_diacritic in [(True, True), (False, True), (True, False), (False, False)]:
                    kwargs = {"case_ending": case_ending, "no_diacritic": no_diacritic}
                    der = evaluator.calculate_der(paths["gold"], paths[name], arabic, classes, "Fadel", **kwargs)
                    wer = evaluator.calculate_wer(paths["gold"], paths[name], arabic, classes, "Fadel", **kwargs)
                    results.append({"case_endings": case_ending, "include_no_diacritic": no_diacritic, "DER_percent": der, "WER_percent": wer})
                report["fadel_evaluator"]["results"][name] = results
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
