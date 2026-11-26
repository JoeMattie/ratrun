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
bindgen=${WASM_BINDGEN:-"$root/.tools/bin/wasm-bindgen"}
if [ ! -x "$bindgen" ] || [ "$("$bindgen" --version)" != "wasm-bindgen $bindgen_version" ]; then
    if [ -n "${WASM_BINDGEN:-}" ]; then
        printf 'Expected wasm-bindgen %s at %s\n' "$bindgen_version" "$bindgen" >&2
        exit 1
    fi
    cargo install --locked --root "$root/.tools" --version "$bindgen_version" wasm-bindgen-cli
fi

if [ -z "${WASM_OPT:-}" ]; then
    npm ci --prefix web --ignore-scripts --include=dev
    WASM_OPT="$root/web/node_modules/.bin/wasm-opt"
fi
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS=${CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS:--C target-feature=+simd128}
cargo build --profile web-release --locked --target "$target" --no-default-features --features web

dist="$root/web/dist"
mkdir -p "$dist"
"$bindgen" "$CARGO_TARGET_DIR/$target/web-release/ratrun.wasm" \
    --target web --no-typescript --remove-name-section --out-dir "$dist"
"$WASM_OPT" -O3 "$dist/ratrun_bg.wasm" -o "$dist/ratrun_bg.wasm"
cp LICENSE web/index.html web/game.js web/canvas.js "$dist/"
cp web/manifest.webmanifest "$dist/"
if [ -d web/icons ]; then
    # The PNGs only: make_icons.py is the generator, not a page asset.
    mkdir -p "$dist/icons"
    cp web/icons/*.png "$dist/icons/"
else
    printf 'web/icons is missing; the page links icons the art pass has not delivered yet\n' >&2
fi
# The service worker's cache name is the Cargo version, so a new build installs
# a new cache and activate deletes the old ones.
version=$(awk -F'"' '/^version = / { print $2; exit }' Cargo.toml)
sed "s/__RATRUN_CACHE__/ratrun-$version/" web/sw.js > "$dist/sw.js"
# Remove assets from an in-place rebuild of the former browser renderer.
rm -f "$dist/vendor/xterm.css" "$dist/vendor/xterm.mjs" "$dist/vendor/addon-fit.mjs" \
    "$dist/vendor/xterm.LICENSE" "$dist/vendor/addon-fit.LICENSE"
