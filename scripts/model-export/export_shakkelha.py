#!/usr/bin/env python3
"""Export the pinned Shakkelha checkpoint without TensorFlow or Python serving.

The graph and metadata deliberately reproduce the researched release artifact
byte for byte. Sigmoid restores the original CuDNN gate semantics; the CPU
checkpoint's serialized hard-sigmoid setting does not describe those semantics.
"""

import argparse
import hashlib
import json
from pathlib import Path

import h5py
import numpy as np
import onnx
from onnx import TensorProto as T, helper as H, numpy_helper as N

COMMIT = "2214779456a905964d764dc4921fb70aa0d0aab6"
CHECKPOINT = "models/rnn_models/3_normalized_model/big_data/lstm/avg_20.ckpt"
CHECKPOINT_SHA256 = "46b18b13c3e3f9a45434d1ba70086e0ab6556456b6e3282b02bfb16ec4eed1ca"
GPU_SHA256 = "d737aafeaa5b9ac277a306219ad30d425e103993351cbf288707af97fefe0aa8"
MODEL_SHA256 = "41ed03075e80847aaef2bfdb65f123b7f225ff6a179a507b1ec3e6152184fa71"


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read_checkpoint(path, expected_hash):
    if sha256(path) != expected_hash:
        raise ValueError(f"checkpoint SHA256 mismatch: {path}")
    with h5py.File(path) as file:
        config = json.loads(file.attrs["model_config"])
        arrays = {}
        file["model_weights"].visititems(
            lambda key, value: arrays.__setitem__(key, value[()])
            if isinstance(value, h5py.Dataset) else None
        )
    return config, arrays


def load_weights(path):
    config, arrays = read_checkpoint(path, CHECKPOINT_SHA256)
    embedding = next(value for key, value in arrays.items() if key.endswith("/embeddings:0"))
    recurrent, dense = [], []
    for layer in config["config"]["layers"]:
        name = layer["name"]
        if layer["class_name"] == "Bidirectional":
            directions = []
            for direction in ["forward", "backward"]:
                selected = {
                    key.rsplit("/", 1)[1]: value for key, value in arrays.items()
                    if key.startswith(name + "/") and "/" + direction + "_" in key
                }
                directions.append([selected[key + ":0"] for key in ["kernel", "recurrent_kernel", "bias"]])
            recurrent.append(directions)
        elif layer["class_name"] == "TimeDistributed":
            selected = {key.rsplit("/", 1)[1]: value for key, value in arrays.items() if key.startswith(name + "/")}
            dense.append([selected[key + ":0"] for key in ["kernel", "bias"]])
    if embedding.shape != (91, 25) or len(recurrent) != 2 or len(dense) != 3:
        raise ValueError("unexpected checkpoint architecture")
    return embedding, recurrent, dense


def export(checkpoint, output):
    embedding, recurrent, dense = load_weights(checkpoint)
    nodes, initializers = [], []

    def constant(name, value):
        initializers.append(N.from_array(np.asarray(value), name))
        return name

    def node(operation, inputs, name, **attributes):
        nodes.append(H.make_node(operation, inputs, [name], **attributes))
        return name

    def reorder(array):
        # Keras gate order i,f,c,o -> ONNX gate order i,o,f,c.
        return np.concatenate([np.split(array, 4, axis=0)[i] for i in [0, 3, 1, 2]], axis=0)

    value = node("Gather", [constant("embedding", embedding), "input_ids"], "embedded", axis=0)
    value = node("Transpose", [value], "sequence_major", perm=[1, 0, 2])
    for index, directions in enumerate(recurrent):
        prefix = "lstm" + str(index)
        w = np.stack([reorder(weights[0].T) for weights in directions])
        r = np.stack([reorder(weights[1].T) for weights in directions])
        b = np.stack([np.concatenate([reorder(weights[2]), np.zeros_like(weights[2])]) for weights in directions])
        value = node("LSTM", [value, constant(prefix + "_W", w), constant(prefix + "_R", r), constant(prefix + "_B", b)], prefix + "_Y", hidden_size=256, direction="bidirectional", activations=["Sigmoid", "Tanh", "Tanh"] * 2)
        value = node("Transpose", [value], prefix + "_ordered", perm=[0, 2, 1, 3])
        value = node("Reshape", [value, constant(prefix + "_shape", np.array([0, 0, 512], dtype=np.int64))], prefix + "_joined")
    value = node("Transpose", [value], "batch_major", perm=[1, 0, 2])
    for index, (w, b) in enumerate(dense):
        value = node("MatMul", [value, constant("dense" + str(index) + "_W", w)], "matmul" + str(index))
        value = node("Add", [value, constant("dense" + str(index) + "_B", b)], "biased" + str(index))
        if index < 2:
            value = node("Relu", [value], "relu" + str(index))
    node("Softmax", [value], "probabilities", axis=-1)
    graph = H.make_graph(nodes, "shakkelha_normalized_big_avg20", [H.make_tensor_value_info("input_ids", T.INT64, ["batch", "sequence"])], [H.make_tensor_value_info("probabilities", T.FLOAT, ["batch", "sequence", 19])], initializers)
    model = H.make_model(graph, producer_name="tashkeel-feasibility-research", opset_imports=[H.make_opsetid("", 17)], ir_version=8)
    H.set_model_props(model, {
        "source_commit": COMMIT,
        "source_checkpoint": CHECKPOINT,
        "gate_activation": "Sigmoid",
        "note": "HardSigmoid matches serialized CPU checkpoint; Sigmoid restores CuDNN gate semantics, to validate against original GPU output.",
    })
    onnx.checker.check_model(model)
    output.parent.mkdir(parents=True, exist_ok=True)
    onnx.save(model, output)
    actual_hash = sha256(output)
    if actual_hash != MODEL_SHA256:
        raise ValueError(f"export differs from pinned artifact: {actual_hash}; check pinned dependencies")
    return actual_hash


def compare_gpu_checkpoint(cpu_path, gpu_path):
    """Apply Keras's legacy CuDNN-to-LSTM weight conversion, not live GPU inference."""
    _, gpu = read_checkpoint(gpu_path, GPU_SHA256)
    _, cpu = read_checkpoint(cpu_path, CHECKPOINT_SHA256)
    comparisons = []
    for (gpu_name, value), (cpu_name, expected) in zip(sorted(gpu.items()), sorted(cpu.items()), strict=True):
        if "bidirectional" in gpu_name:
            if gpu_name.endswith("/kernel:0"):
                value = np.hstack([gate.T.reshape(gate.shape, order="F") for gate in np.hsplit(value, 4)])
            elif gpu_name.endswith("/recurrent_kernel:0"):
                value = np.hstack([gate.T for gate in np.hsplit(value, 4)])
            else:
                value = sum(np.split(value, 2))
        equal = bool(np.array_equal(value, expected))
        comparisons.append({"gpu": gpu_name, "cpu": cpu_name, "exact_equal": equal, "max_abs_error": float(np.max(np.abs(value - expected)))})
    if not all(item["exact_equal"] for item in comparisons):
        raise ValueError("GPU and CPU checkpoint weights differ after Keras conversion")
    return comparisons


def reference(ids, weights):
    """Independent NumPy recurrence in Keras gate order, for numerical checks."""
    embedding, recurrent, dense = weights
    value = embedding[ids]
    for directions in recurrent:
        outputs = []
        for direction, (w, r, b) in enumerate(directions):
            hidden = np.zeros((value.shape[0], 256), np.float32)
            cell = hidden.copy()
            output = np.empty((value.shape[0], value.shape[1], 256), np.float32)
            positions = range(value.shape[1]) if direction == 0 else range(value.shape[1] - 1, -1, -1)
            for position in positions:
                i, f, g, o = np.split(value[:, position] @ w + hidden @ r + b, 4, axis=-1)
                sigmoid = lambda x: 1 / (1 + np.exp(-np.clip(x, -80, 80)))
                cell = sigmoid(f) * cell + sigmoid(i) * np.tanh(g)
                hidden = sigmoid(o) * np.tanh(cell)
                output[:, position] = hidden
            outputs.append(output)
        value = np.concatenate(outputs, axis=-1)
    for index, (w, b) in enumerate(dense):
        value = value @ w + b
        if index < 2:
            value = np.maximum(value, 0)
    exponentials = np.exp(value - value.max(-1, keepdims=True))
    return exponentials / exponentials.sum(-1, keepdims=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--gpu-checkpoint", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    report = {"model_sha256": export(args.checkpoint, args.output), "checkpoint_sha256": CHECKPOINT_SHA256}
    if args.gpu_checkpoint:
        report["gpu_weight_comparison"] = compare_gpu_checkpoint(args.checkpoint, args.gpu_checkpoint)
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
