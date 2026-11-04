#!/usr/bin/env bash
# Tag the checked-out main commit with the next free vX.Y.Z and push it.
#
# Usage: scripts/tag-release.sh   (from a full clone whose origin is the repository)
#
# .github/workflows/tag-release.yml runs this for each main commit that passes
# the checks. After a GITHUB_TOKEN push it dispatches release.yml for the
# version that this script prints. The script
# prints the version on stdout and nothing else. It prints nothing when it
# skips. Progress and git output go to stderr.
#
# - Skip (exit 0) when the incoming commit is a `chore(release):` commit, or
#   when HEAD already has a `v*` tag.
# - Start from the version in crates/workspace-status/Cargo.toml and bump the
#   patch until `v<version>` is not a tag, local or on origin. A taken tag,
#   also one on a commit that is on no branch, is skipped and left unchanged.
# - When the version changes, bump both crates and their Cargo.lock entries in
#   one `chore(release): v<version>` commit.
# - Push main and the tag with `--atomic`. When origin refuses main (a newer
#   merge landed first), it refuses the tag too, and the run for the newer
#   merge tags that version instead.
set -euo pipefail

# Keep stdout for the chosen version.
exec 3>&1 1>&2

cd "$(git rev-parse --show-toplevel)"
git fetch --tags origin

msg=$(git log -1 --pretty=%s)
if [[ "$msg" == "chore(release):"* ]]; then
  echo "skip: incoming commit is already chore(release):"
  exit 0
fi

if exact=$(git describe --exact-match --tags HEAD 2>/dev/null); then
  if [[ "$exact" == v* ]]; then
    echo "skip: HEAD is already tagged $exact"
    exit 0
  fi
fi

current=$(awk '/^version = / { gsub(/"/, "", $3); print $3; exit }' crates/workspace-status/Cargo.toml)
if [[ -z "$current" ]]; then
  echo "could not read crates/workspace-status/Cargo.toml version" >&2
  exit 1
fi

version="$current"
while git rev-parse -q --verify "refs/tags/v${version}" >/dev/null; do
  echo "tag v${version} is taken"
  IFS=. read -r major minor patch <<<"$version"
  version="${major}.${minor}.$((patch + 1))"
done

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"

if [[ "$version" != "$current" ]]; then
  for f in crates/workspace-status/Cargo.toml crates/workspace-status-graph/Cargo.toml; do
    sed -i "s/^version = \"${current}\"/version = \"${version}\"/" "$f"
  done
  # Keep workspace package versions in Cargo.lock in lockstep.
  awk -v old="$current" -v new="$version" '
    $0 == "name = \"workspace-status\"" { hit = 1 }
    $0 == "name = \"workspace-status-graph\"" { hit = 1 }
    hit && $0 ~ /^version = "/ {
      sub("version = \"" old "\"", "version = \"" new "\"")
      hit = 0
    }
    { print }
  ' Cargo.lock > Cargo.lock.next
  mv Cargo.lock.next Cargo.lock
  git add crates/workspace-status/Cargo.toml crates/workspace-status-graph/Cargo.toml Cargo.lock
  git commit -m "chore(release): v${version}"
fi

git tag -a "v${version}" -m "v${version}"
git push --atomic origin HEAD:main "v${version}"
echo "$version" >&3
