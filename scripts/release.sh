#!/bin/zsh
# Prepare a release: bump the version everywhere, commit, and tag.
#
#   scripts/release.sh 0.2.0
#
# Pushing the tag (printed at the end) makes GitHub Actions build and publish
# it (.github/workflows/release.yml); installed copies then offer the update.
set -e
cd ${0:A:h:h}
version=${1#v}
if [[ ! $version =~ '^[0-9]+\.[0-9]+\.[0-9]+$' ]]; then
  echo "usage: scripts/release.sh <major.minor.patch>"; exit 1
fi
current=$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"([0-9.]+)".*/\1/')
if [[ $(printf '%s\n%s\n' $current $version | sort -V | tail -1) != $version || $current == $version ]]; then
  echo "version must be newer than the current $current"; exit 1
fi
[[ $(git branch --show-current) == main ]] || { echo "release from main"; exit 1; }
[[ -z $(git status --porcelain) ]] || { echo "commit or stash your changes first"; exit 1; }
git rev-parse -q --verify "refs/tags/v$version" >/dev/null && { echo "tag v$version already exists"; exit 1; }

# tauri.conf.json is what the app and updater report; keep the others in step.
for file in src-tauri/tauri.conf.json package.json; do
  perl -pi -e 's/("version": )"[0-9.]+"/$1"'$version'"/ if $. < 6' $file
done
perl -pi -e 's/^version = "[0-9.]+"/version = "'$version'"/ if $. < 5' src-tauri/Cargo.toml
(cd src-tauri && cargo update -q -p ternt --offline 2>/dev/null || cargo update -q -p ternt)

git add src-tauri/tauri.conf.json package.json src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -q -m "Release v$version"
git tag -a "v$version" -m "Ternt $version"
echo "Tagged v$version ($current → $version). Publish it with:"
echo
echo "  git push origin main v$version"
