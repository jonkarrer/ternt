#!/bin/zsh
# Prepare a release: bump the version everywhere, commit, and tag.
#
#   scripts/release.sh 0.2.0          # explicit version
#   scripts/release.sh minor          # or patch / major: computed from the current version
#   scripts/release.sh --next minor   # only print what that version would be
#
# `just upload` runs this when HEAD isn't tagged yet (asking patch/minor/major),
# then builds and publishes; installed copies then offer the update.
set -e
cd ${0:A:h:h}
current=$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"([0-9.]+)".*/\1/')
print_only=; [[ $1 == --next ]] && { print_only=1; shift; }
version=${1#v}
parts=(${(s:.:)current})
case $version in
  major) version=$((parts[1] + 1)).0.0 ;;
  minor) version=$parts[1].$((parts[2] + 1)).0 ;;
  patch) version=$parts[1].$parts[2].$((parts[3] + 1)) ;;
esac
if [[ ! $version =~ '^[0-9]+\.[0-9]+\.[0-9]+$' ]]; then
  echo "usage: scripts/release.sh <major.minor.patch> | major | minor | patch"; exit 1
fi
[[ -z $print_only ]] || { echo $version; exit 0; }
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
echo "Tagged v$version ($current → $version). Publish it with: just upload"
