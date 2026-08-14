#!/usr/bin/env bash
# Explicit, resumable F2 vision-model download. This script never starts by itself.
set -euo pipefail

destination="${JARVIS_VISION_MODEL_DIR:-/home/mehmet/jarvis/models/Qwen2.5-VL-3B-Instruct-GGUF}"
mkdir -p "$destination"

download() {
    local file="$1"
    local url="$2"
    curl --fail --location --continue-at - --retry 20 --retry-all-errors --retry-delay 5 \
        --speed-limit 1 --speed-time 60 --output "$destination/$file" "$url"
}

download \
    "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf" \
    "https://huggingface.co/ggml-org/Qwen2.5-VL-3B-Instruct-GGUF/resolve/main/Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf"
download \
    "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf" \
    "https://huggingface.co/ggml-org/Qwen2.5-VL-3B-Instruct-GGUF/resolve/main/mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf"
