#!/usr/bin/env bash
# Download a small instruct-tuned GGUF model for NPC dialogue.
# Qwen2.5-0.5B-Instruct Q4_K_M ≈ 400 MB — a good starting point: coherent,
# fast on CPU. Swap MODEL_URL for any other GGUF chat model if you like
# (e.g. SmolLM2-360M-Instruct for something even lighter).
set -euo pipefail
DIR="$(cd "$(dirname "$0")/.." && pwd)/models"
mkdir -p "$DIR"
MODEL_URL="https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf"
OUT="$DIR/qwen2.5-0.5b-instruct-q4_k_m.gguf"
if [ -f "$OUT" ]; then
    echo "Model already present: $OUT"
    exit 0
fi
echo "Downloading to $OUT …"
curl -L --fail --progress-bar -o "$OUT.part" "$MODEL_URL"
mv "$OUT.part" "$OUT"
echo "Done. Point Database → LLM → Model at: $OUT"
