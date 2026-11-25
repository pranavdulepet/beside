#!/bin/bash
# Restore the pinned Apple Silicon llama.cpp runtime. An optional local archive
# supports offline builds. Only the libraries needed by the server are bundled.
set -euo pipefail
cd "$(dirname "$0")/.."
stage=$(mktemp -d "${TMPDIR:-/tmp}/sumi-runtime-XXXXXXXX")
trap 'rm -rf "$stage"' EXIT
archive=${1:-"$stage/runtime.tar.gz"}
if [[ $# -eq 0 ]]; then
  curl --fail --location --retry 2 --output "$archive" \
    https://github.com/ggml-org/llama.cpp/releases/download/b12633/llama-b12633-bin-macos-arm64.tar.gz
fi
digest=$(shasum -a 256 "$archive" | cut -d' ' -f1)
[[ "$digest" == 99657876053df9004753d92075813bbb3b484e79f8a402809b51d00c8c540062 ]] || { echo "Runtime checksum mismatch" >&2; exit 1; }
tar -xf "$archive" -C "$stage"
runtime="$stage/llama-b12633"
for file in llama-server libggml{,-base,-blas,-cpu,-metal,-rpc}.1.dylib libllama{,-common}.1.dylib libllama-server-impl.dylib libmtmd.1.dylib; do
  install -m 755 "$runtime/$file" "src-tauri/runtime/$file"
done
src-tauri/runtime/llama-server --version
