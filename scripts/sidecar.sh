#!/bin/zsh
# Build the agent sidecar and put it where Tauri's build script and bundler
# look: src-tauri/binaries/ternt-agent-<target triple>.
#
#   scripts/sidecar.sh [--release] [stub|tokidoki]
#
#   stub       src-tauri/agent: canned replies, no key needed (the default; the
#              e2e suites are written against it)
#   tokidoki   tokidoki-serve, the real agent, built from the tokidoki repo at
#              $TOKIDOKI_SRC (default: ../tokidoki, beside this repo)
#   --release  both Apple targets plus a universal binary (scripts/publish.sh),
#              with Apple's toolchain so it links only system libraries
#
# TERNT_SIDECAR=stub|tokidoki picks the flavor for callers that don't say
# (just dev, e2e/run.sh, scripts/publish.sh).
#
# Ternt itself can't compile until the file exists (tauri-build checks
# bundle.externalBin), which is why the stub is its own crate.
set -e
root=${0:A:h:h}
cd $root/src-tauri
mkdir -p binaries

release=
flavor=${TERNT_SIDECAR:-stub}
for arg in "$@"; do
  case $arg in
    --release) release=1 ;;
    stub|tokidoki) flavor=$arg ;;
    "") ;;
    *) echo "sidecar: unknown argument '$arg' (--release, stub, tokidoki)" >&2; exit 2 ;;
  esac
done

case $flavor in
  stub) package=ternt-agent; bin=ternt-agent; manifest=Cargo.toml; out=target ;;
  tokidoki)
    src=${TOKIDOKI_SRC:-${root:h}/tokidoki}
    [[ -f $src/Cargo.toml ]] || { echo "sidecar: no tokidoki checkout at $src (set TOKIDOKI_SRC)" >&2; exit 1; }
    package=tokidoki; bin=tokidoki-serve; manifest=$src/Cargo.toml; out=$src/target ;;
  *) echo "sidecar: unknown flavor '$flavor' (stub or tokidoki)" >&2; exit 2 ;;
esac

host=$(rustc -vV | sed -n 's/^host: //p')
if [[ -n $release ]]; then
  # Apple's toolchain whatever shell this runs from: a Nix compiler would link
  # /nix/store libraries other Macs don't have (SQLite is compiled in).
  export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
  [[ -d $DEVELOPER_DIR ]] || export DEVELOPER_DIR=$(/usr/bin/xcode-select -p)
  export PATH=/usr/bin:/usr/sbin:$PATH
  export CC=/usr/bin/clang CXX=/usr/bin/clang++ AR=/usr/bin/ar
  export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=/usr/bin/clang
  export CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=/usr/bin/clang
  export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-14.0} # bundle.macOS.minimumSystemVersion
  unset SDKROOT NIX_CFLAGS_COMPILE NIX_LDFLAGS NIX_ENFORCE_PURITY \
    NIX_CFLAGS_COMPILE_FOR_TARGET NIX_LDFLAGS_FOR_TARGET LD LIBRARY_PATH
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build -q --release --manifest-path $manifest -p $package --bin $bin --target $target
    cp $out/$target/release/$bin binaries/ternt-agent-$target
  done
  lipo -create -output binaries/ternt-agent-universal-apple-darwin \
    binaries/ternt-agent-aarch64-apple-darwin binaries/ternt-agent-x86_64-apple-darwin
  if otool -L binaries/ternt-agent-universal-apple-darwin | grep -E '^\s+/(nix|opt|usr/local)/'; then
    echo "sidecar: links non-system libraries (see above)" >&2; exit 1
  fi
  echo "sidecar ($flavor): binaries/ternt-agent-{aarch64,x86_64,universal}-apple-darwin"
else
  cargo build -q --manifest-path $manifest -p $package --bin $bin
  cp $out/debug/$bin binaries/ternt-agent-$host
  echo "sidecar ($flavor): binaries/ternt-agent-$host (debug)"
fi
