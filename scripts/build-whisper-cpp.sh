#!/usr/bin/env sh
set -eu

# One reproducible CPU runtime/model for both indexer images. Run in a Debian
# bookworm build stage with build-essential, cmake, git, curl and ca-certificates.
destination="${1:-/opt/ytsearch-asr}"
source_commit=d1be6fde11ac6e0407606b4e42fe72d34add8037 # whisper.cpp v1.9.5
model_revision=5359861c739e955e79d9a303bcbc70fb988958b1
# SHA256 from the maintainer's ggml-base.en.bin Git LFS object at this revision:
# https://huggingface.co/api/models/ggerganov/whisper.cpp/tree/5359861c739e955e79d9a303bcbc70fb988958b1
model_sha256=a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
trap 'exit 130' INT TERM

git init -q "$work/source"
git -C "$work/source" remote add origin https://github.com/ggml-org/whisper.cpp.git
git -C "$work/source" fetch --depth 1 origin "$source_commit"
git -C "$work/source" checkout -q --detach FETCH_HEAD
test "$(git -C "$work/source" rev-parse HEAD)" = "$source_commit"
cmake -S "$work/source" -B "$work/build" \
  -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=ON \
  -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON \
  -DGGML_BACKEND_DIR="$destination/lib" \
  -DWHISPER_BUILD_IS_DEV=OFF -DWHISPER_BUILD_TESTS=OFF \
  -DWHISPER_BUILD_SERVER=OFF -DWHISPER_CURL=OFF \
  -DGGML_NATIVE=OFF -DGGML_OPENMP=OFF -DGGML_CUDA=OFF \
  -DGGML_METAL=OFF -DGGML_VULKAN=OFF \
  -DGGML_SSE42=OFF -DGGML_AVX=OFF -DGGML_AVX2=OFF \
  -DGGML_BMI2=OFF -DGGML_FMA=OFF -DGGML_F16C=OFF
cmake --build "$work/build" --target whisper-cli --parallel "${ASR_BUILD_JOBS:-2}"

install -d "$destination/bin" "$destination/libexec" "$destination/lib" "$destination/share"
install -m 0755 "$work/build/bin/whisper-cli" "$destination/libexec/whisper-cli"
find "$work/build" \( -type f -o -type l \) -name '*.so*' -exec cp -P {} "$destination/lib/" \;
cat > "$destination/bin/whisper-cli" <<'WRAPPER'
#!/bin/sh
set -eu
runtime="$(dirname "$(dirname "$(readlink -f "$0")")")"
export LD_LIBRARY_PATH="$runtime/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$runtime/libexec/whisper-cli" "$@"
WRAPPER
chmod 0755 "$destination/bin/whisper-cli"
install -m 0644 "$work/source/LICENSE" "$destination/share/whisper.cpp-LICENSE"
curl --fail --location --retry 3 --max-time 600 \
  "https://huggingface.co/ggerganov/whisper.cpp/resolve/$model_revision/ggml-base.en.bin" \
  --output "$destination/share/ggml-base.en.bin"
printf '%s  %s\n' "$model_sha256" "$destination/share/ggml-base.en.bin" | sha256sum --check
curl --fail --location --retry 3 --max-time 60 \
  https://raw.githubusercontent.com/openai/whisper/86098128c0b4f24f0e2aa2994de830614b474227/LICENSE \
  --output "$destination/share/whisper-model-LICENSE"
chmod 0644 "$destination/share/ggml-base.en.bin" "$destination/share/whisper-model-LICENSE"
printf 'whisper.cpp=%s\nmodel_revision=%s\nmodel_sha256=%s\n' \
  "$source_commit" "$model_revision" "$model_sha256" > "$destination/share/provenance"
"$destination/bin/whisper-cli" --help >/dev/null 2>&1
