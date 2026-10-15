#!/usr/bin/env python3
"""Download pinned CPU-only semantic pilot assets without credentials or Docker."""

import argparse
import hashlib
import json
import os
import pathlib
import platform
import tarfile
import tempfile
import urllib.request


MODEL_REVISION = "5090578d9565bb06545b4552f76e6bc2c93e4a66"
MODEL_BASE = (
    "https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX/resolve/"
    + MODEL_REVISION
)
MODEL_FILES = {
    "onnx/model_q4.onnx": "ad1dfee81a70f7944b9b9d1cc6e48075b832881cf33fab2f2b248be78f3f0043",
    "onnx/model_q4.onnx_data": "599962c3143b040de2dd05e5975be3e9091dd067cacc6a8f7186e3203bab9e02",
    "tokenizer.json": "4dda02faaf32bc91031dc8c88457ac272b00c1016cc679757d1c441b248b9c47",
    "tokenizer_config.json": "3ca953eea6c3c9fcda9cf3df22949ff18b216f7c74bd6459230f3f1013953f3a",
    "config.json": "6e1f06404b7163e0325ed2ea3e6781cde50f4a50b31780a95ad0d30e8404d77b",
    "special_tokens_map.json": "2f7b0adf4fb469770bb1490e3e35df87b1dc578246c5e7e6fc76ecf33213a397",
}
ONNX_VERSION = "1.31.0"
ONNX_SHA256 = "cc5c72baf5ae5c8238a6841f0897227be2d02826b9cf98eaf02fdefaeeb03a57"
QDRANT_VERSION = "1.19.2"
QDRANT_SHA256 = "34a788a09a4cb278b5c6d2af96d9a6db30c8c7e88ae3550455f7a4d425bb8d4b"


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def download(url, destination, expected):
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.is_file() and sha256(destination) == expected:
        print("verified", destination, flush=True)
        return
    partial = None
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "ytsearch-semantic-setup"})
        with urllib.request.urlopen(request, timeout=60) as response:
            with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as output:
                partial = pathlib.Path(output.name)
                digest = hashlib.sha256()
                while block := response.read(1024 * 1024):
                    digest.update(block)
                    output.write(block)
        if digest.hexdigest() != expected:
            raise ValueError("SHA-256 mismatch for " + destination.name)
        os.replace(partial, destination)
        print("downloaded", destination, destination.stat().st_size, "bytes", flush=True)
    finally:
        if partial is not None:
            partial.unlink(missing_ok=True)


def unpack(archive, destination):
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive) as source:
        source.extractall(destination, filter="data")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--directory",
        type=pathlib.Path,
        default=pathlib.Path(__file__).resolve().parents[1] / "data" / "semantic",
    )
    args = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        parser.error("the pinned native binaries support Linux x86_64; provide an appropriate ONNX Runtime/Qdrant build on other platforms")
    root = args.directory.resolve()
    for name, digest in MODEL_FILES.items():
        download(MODEL_BASE + "/" + name, root / "model" / name, digest)
    onnx_name = "onnxruntime-linux-x64-" + ONNX_VERSION
    onnx_archive = root / ".downloads" / (onnx_name + ".tgz")
    download(
        "https://github.com/microsoft/onnxruntime/releases/download/v" + ONNX_VERSION + "/" + onnx_archive.name,
        onnx_archive,
        ONNX_SHA256,
    )
    unpack(onnx_archive, root / "runtime")
    qdrant_archive = root / ".downloads" / "qdrant-x86_64-unknown-linux-gnu.tar.gz"
    download(
        "https://github.com/qdrant/qdrant/releases/download/v" + QDRANT_VERSION + "/" + qdrant_archive.name,
        qdrant_archive,
        QDRANT_SHA256,
    )
    unpack(qdrant_archive, root / "qdrant")
    manifest = {
        "model_repository": "onnx-community/embeddinggemma-300m-ONNX",
        "model_revision": MODEL_REVISION,
        "model_files_sha256": MODEL_FILES,
        "onnxruntime_version": ONNX_VERSION,
        "qdrant_version": QDRANT_VERSION,
    }
    (root / "assets.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({
        "model_directory": str(root / "model"),
        "ORT_DYLIB_PATH": str(root / "runtime" / onnx_name / "lib" / ("libonnxruntime.so." + ONNX_VERSION)),
        "qdrant_binary": str(root / "qdrant" / "qdrant"),
    }, indent=2))


if __name__ == "__main__":
    main()
