#!/bin/zsh
# Copy tokidoki's protocol into Ternt's host code: the Rust types
# (tokidoki/crates/interface/src/protocol.rs) and the TypeScript generated from them
# (`just types` in tokidoki/). One source of truth; run this after the protocol changes.
# $TOKIDOKI_SRC points at another tokidoki tree.
set -e
root=${0:A:h:h}
src=${TOKIDOKI_SRC:-$root/tokidoki}
[[ -f $src/crates/interface/src/protocol.rs ]] || { echo "sync-protocol: no tokidoki tree at $src (set TOKIDOKI_SRC)" >&2; exit 1; }
{
  echo "//! Tokidoki's protocol, copied from tokidoki/crates/interface/src/protocol.rs by"
  echo "//! scripts/sync-protocol.sh; don't edit it here. The same types as TypeScript are"
  echo "//! src/agent/protocol.ts. The sidecar's stub compiles this file too."
  echo "#![allow(dead_code)] // Ternt uses what it needs of tokidoki's API."
  echo
  # The ts-rs derives are tokidoki's own build; Ternt has no such feature.
  grep -v '^#\[cfg_attr(feature = "ts"' $src/crates/interface/src/protocol.rs
} > $root/src-tauri/src/agent/protocol.rs
{
  echo "// Tokidoki's protocol, copied from tokidoki/crates/interface/protocol.ts (generated there"
  echo "// by \`just types\`) by scripts/sync-protocol.sh; don't edit it here."
  echo "// src-tauri/src/agent/protocol.rs is the same set of types in Rust."
  echo "//"
  cat $src/crates/interface/protocol.ts
} > $root/src/agent/protocol.ts
echo "protocol: synced from $src"
