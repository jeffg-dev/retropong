#!/bin/sh
set -eu
web_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$web_dir/../.." && pwd)
cd "$repo_dir"
cargo build --release --target wasm32-unknown-unknown -p superpong-web
mkdir -p "$web_dir/public"
cp "${CARGO_TARGET_DIR:-$repo_dir/target}/wasm32-unknown-unknown/release/superpong_web.wasm" "$web_dir/public/retropong.wasm"
cp "$repo_dir/game/superpong/src/shaders/crt.frag" "$web_dir/public/crt.frag"
cargo run --release -p superpong -- --export-sounds "$web_dir/public/sounds"
printf 'Built %s/public/retropong.wasm\n' "$web_dir"
