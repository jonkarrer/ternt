#!/bin/zsh
# Build the agent sidecar (src-tauri/agent) and put it where Tauri's build
# script and bundler look: src-tauri/binaries/ternt-agent-<target triple>.
#
#   scripts/sidecar.sh             debug, for this machine (just dev, e2e, cargo check)
#   scripts/sidecar.sh --release   release, both Apple targets plus a universal
#                                  binary (scripts/publish.sh)
#
# Ternt itself can't compile until the file exists (tauri-build checks
# bundle.externalBin), which is why the sidecar is its own crate.
set -e
cd ${0:A:h:h}/src-tauri
mkdir -p binaries
host=$(rustc -vV | sed -n 's/^host: //p')
if [[ $1 == --release ]]; then
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build -q --release -p ternt-agent --target $target
    cp target/$target/release/ternt-agent binaries/ternt-agent-$target
  done
  lipo -create -output binaries/ternt-agent-universal-apple-darwin \
    binaries/ternt-agent-aarch64-apple-darwin binaries/ternt-agent-x86_64-apple-darwin
  echo "sidecar: binaries/ternt-agent-{aarch64,x86_64,universal}-apple-darwin"
else
  cargo build -q -p ternt-agent
  cp target/debug/ternt-agent binaries/ternt-agent-$host
  echo "sidecar: binaries/ternt-agent-$host (debug)"
fi
