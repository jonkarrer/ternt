#!/bin/zsh
# Copy tokidoki's protocol into Ternt: the Rust types (crates/interface/src/protocol.rs)
# and the TypeScript generated from them (`just types` there). One source of truth;
# run this after the protocol changes in tokidoki. $TOKIDOKI_SRC defaults to ../tokidoki.
set -e
root=${0:A:h:h}
src=${TOKIDOKI_SRC:-${root:h}/tokidoki}
[[ -f $src/crates/interface/src/protocol.rs ]] || { echo "sync-protocol: no tokidoki checkout at $src (set TOKIDOKI_SRC)" >&2; exit 1; }
commit=$(git -C $src rev-parse --short HEAD 2>/dev/null || echo unknown)
{
  echo "//! Tokidoki's protocol, copied from crates/interface/src/protocol.rs in the tokidoki"
  echo "//! repo (commit $commit) by scripts/sync-protocol.sh; don't edit it here. The same"
  echo "//! types as TypeScript are src/agent/protocol.ts. The sidecar's stub compiles this file too."
  echo "#![allow(dead_code)] // Ternt uses what it needs of tokidoki's API."
  echo
  # The ts-rs derives are tokidoki's own build; Ternt has no such feature.
  grep -v '^#\[cfg_attr(feature = "ts"' $src/crates/interface/src/protocol.rs
} > $root/src-tauri/src/agent/protocol.rs
{
  echo "// Tokidoki's protocol, copied from crates/interface/protocol.ts in the tokidoki repo"
  echo "// (commit $commit; generated there by \`just types\`) by scripts/sync-protocol.sh; don't"
  echo "// edit it here. src-tauri/src/agent/protocol.rs is the same set of types in Rust."
  echo "//"
  cat $src/crates/interface/protocol.ts
} > $root/src/agent/protocol.ts
echo "protocol: synced from tokidoki @ $commit"
