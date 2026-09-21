#!/bin/bash
# Cross-compile atomrust for arm64 Raspberry Pi OS (trixie) and package it as a .deb.
# Runs on an x86_64 Linux host with podman (or docker). Output: dist/atomrust_<ver>_arm64.deb
#
#   packaging/build-deb.sh                 # full build (with TFLite object detection)
#   packaging/build-deb.sh --no-objdet     # RTSP/MQTT only; much faster first build
#
# The same .deb runs on any 64-bit Pi on trixie (Zero 2 W, 3, 4, 5).
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$PWD
ENGINE=${ENGINE:-$(command -v podman || command -v docker)}
IMAGE=localhost/atomrust-cross:trixie
CARGO_ARGS=(--release --target aarch64-unknown-linux-gnu)
if [[ ${1:-} == --no-objdet ]]; then
	CARGO_ARGS+=(--no-default-features)
fi

git submodule update --init --recursive

# Rebuild the toolchain image only when its recipe changes.
IMAGE_TAG=$(sha256sum packaging/Containerfile | cut -c1-12)
if ! "$ENGINE" image exists "$IMAGE-$IMAGE_TAG" 2>/dev/null; then
	"$ENGINE" build -t "$IMAGE" -t "$IMAGE-$IMAGE_TAG" -f packaging/Containerfile packaging
fi

# Registry and target dir persist between runs so rebuilds are incremental.
mkdir -p "$ROOT/.cross-cache/registry" "$ROOT/.cross-cache/git"
"$ENGINE" run --rm \
	-v "$ROOT:/src:Z" \
	-v "$ROOT/.cross-cache/registry:/opt/cargo/registry:Z" \
	-v "$ROOT/.cross-cache/git:/opt/cargo/git:Z" \
	-w /src \
	-e CARGO_TERM_COLOR=always \
	"$IMAGE-$IMAGE_TAG" \
	bash -euo pipefail -c "
		cargo build ${CARGO_ARGS[*]}
		packaging/assemble-deb.sh target/aarch64-unknown-linux-gnu/release/atomrust dist
	"
ls -l dist/*.deb
