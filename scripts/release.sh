#!/bin/sh
# Cut a release: bump the version, commit, tag vX.Y.Z and push.
# The tag triggers .github/workflows/release.yml, which builds the macOS app,
# publishes the GitHub Release, deploys the landing page and update.json to GitHub Pages.
#
#   scripts/release.sh 0.2.0          # explicit version
#   scripts/release.sh patch|minor|major
#   scripts/release.sh 0.2.0 --yes    # no confirmation prompt
set -e
cd "$(dirname "$0")/.."

die() { echo "release: $*" >&2; exit 1; }

[ -n "$1" ] || die "usage: scripts/release.sh <version|patch|minor|major> [--yes]"
CURRENT=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
IFS=. read -r MAJ MIN PAT <<EOF
$CURRENT
EOF
case "$1" in
  patch) VERSION="$MAJ.$MIN.$((PAT + 1))" ;;
  minor) VERSION="$MAJ.$((MIN + 1)).0" ;;
  major) VERSION="$((MAJ + 1)).0.0" ;;
  *) VERSION="${1#v}" ;;
esac
echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$' || die "invalid version '$VERSION'"
TAG="v$VERSION"

BRANCH=$(git rev-parse --abbrev-ref HEAD)
[ "$BRANCH" = "main" ] || die "releases are cut from main (on '$BRANCH')"
[ -z "$(git status --porcelain)" ] || die "working tree is not clean"
git fetch -q origin --tags
if git rev-parse -q --verify "refs/remotes/origin/main" >/dev/null; then
  [ -z "$(git rev-list HEAD..origin/main)" ] || die "origin/main has commits you don't have; pull first"
fi
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && die "tag $TAG already exists"

echo "Releasing MiniDiff $CURRENT → $VERSION"
if [ "$VERSION" != "$CURRENT" ]; then
  # Only the [package] version (first `version = ` line).
  awk -v v="$VERSION" '!done && /^version = "/ { print "version = \"" v "\""; done = 1; next } { print }' \
    Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
fi
cargo test -q >/dev/null || die "tests failed (Cargo.toml was updated; revert with git checkout Cargo.toml)"

if [ "$2" != "--yes" ]; then
  printf "Commit, tag %s and push to origin? [y/N] " "$TAG"
  read -r answer
  case "$answer" in
    y|Y|yes) ;;
    *) git checkout -q -- Cargo.toml Cargo.lock; die "aborted, version change reverted" ;;
  esac
fi

git add Cargo.toml Cargo.lock
git diff --cached --quiet || git commit -q -m "Release $TAG"
git tag -a "$TAG" -m "MiniDiff $VERSION"
git push -q origin main
git push -q origin "$TAG"

REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || echo "signalwerk/miniDiff")
echo "Pushed $TAG. Follow the build: https://github.com/$REPO/actions"
