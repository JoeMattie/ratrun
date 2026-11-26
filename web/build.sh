#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
root=$(pwd)
target=wasm32-unknown-unknown
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$root/target"}
case "$CARGO_TARGET_DIR" in
    /*) ;;
    *) CARGO_TARGET_DIR="$root/$CARGO_TARGET_DIR" ;;
esac
export CARGO_TARGET_DIR

bindgen_version=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/"/, "", $3); print $3; exit }' Cargo.lock)
bindgen="$root/.tools/bin/wasm-bindgen"
if [ ! -x "$bindgen" ] || [ "$("$bindgen" --version)" != "wasm-bindgen $bindgen_version" ]; then
    cargo install --locked --root "$root/.tools" --version "$bindgen_version" wasm-bindgen-cli
fi

npm ci --prefix web --ignore-scripts
cargo build --release --locked --target "$target" --no-default-features --features web

dist="$root/web/dist"
mkdir -p "$dist/vendor"
"$bindgen" "$CARGO_TARGET_DIR/$target/release/ratrun.wasm" \
    --target web --no-typescript --out-dir "$dist"
cp web/index.html web/game.js "$dist/"
cp web/node_modules/@xterm/xterm/css/xterm.css "$dist/vendor/"
cp web/node_modules/@xterm/xterm/lib/xterm.mjs "$dist/vendor/"
cp web/node_modules/@xterm/addon-fit/lib/addon-fit.mjs "$dist/vendor/"
cp web/node_modules/@xterm/addon-webgl/lib/addon-webgl.mjs "$dist/vendor/"
for package in xterm addon-fit addon-webgl; do
    cp "web/node_modules/@xterm/$package/LICENSE" "$dist/vendor/$package.LICENSE"
done
