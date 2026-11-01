#!/usr/bin/env bash
# Publish a new version: bump the version in the sources, push master,
# merge master into the publish branch, push it and go back to master.
# Usage: scripts/publish.sh <version>   (e.g. scripts/publish.sh 0.5.0)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REMOTE="origin"
MAIN_BRANCH="master"
PUBLISH_BRANCH="publish"
VERSION_REGEX='^[0-9]+\.[0-9]+\.[0-9]+$'

# Print an error message and exit.
die() {
    echo "error: $*" >&2
    exit 1
}

# Fail unless the version is X.Y.Z, we are on master, the repo is clean
# and the publish branch exists.
check_preconditions() {
    local version="$1"
    [[ "$version" =~ $VERSION_REGEX ]] || die "invalid version '$version' (expected X.Y.Z)"
    [[ "$(git rev-parse --abbrev-ref HEAD)" == "$MAIN_BRANCH" ]] || die "must be on branch $MAIN_BRANCH"
    [[ -z "$(git status --porcelain)" ]] || die "working tree is not clean"
    git rev-parse --verify --quiet "refs/heads/$PUBLISH_BRANCH" >/dev/null \
        || git rev-parse --verify --quiet "refs/remotes/$REMOTE/$PUBLISH_BRANCH" >/dev/null \
        || die "branch $PUBLISH_BRANCH does not exist (create it first)"
}

# Replace the version in every file that carries it.
bump_version() {
    local version="$1"

    # Workspace version (inherited by all crates)
    sed -i -E "s/^(version[[:space:]]*=[[:space:]]*)\"[0-9]+\.[0-9]+\.[0-9]+\"/\1\"$version\"/" Cargo.toml

    # macOS bundle version (value is on the line after the key)
    perl -0pi -e "s|(<key>CFBundleShortVersionString</key>\s*<string>)[^<]*(</string>)|\${1}$version\${2}|" \
        extra/macos/GoldenIDE.app/Contents/Info.plist

    # Windows installer
    sed -i -E "s/(<Product [^>]*Version=\")[0-9]+\.[0-9]+\.[0-9]+(\")/\1$version\2/" \
        extra/windows/wix/goldenide.wxs

    # RPM spec
    sed -i -E "s/^(Version:[[:space:]]+)[0-9]+\.[0-9]+\.[0-9]+/\1$version/" goldenide.spec

    # Refresh the workspace crate versions in Cargo.lock
    cargo update --workspace --offline
}

# Run the whole publish flow.
main() {
    local version="${1:-}"
    [[ -n "$version" ]] || die "usage: $0 <version>"

    cd "$ROOT_DIR"
    check_preconditions "$version"

    bump_version "$version"
    git add Cargo.toml Cargo.lock extra/macos/GoldenIDE.app/Contents/Info.plist \
        extra/windows/wix/goldenide.wxs goldenide.spec
    git commit -m "chore: bump version to $version"
    git push "$REMOTE" "$MAIN_BRANCH"

    # Always come back to master, even when a step below fails
    trap 'git checkout "$MAIN_BRANCH"' EXIT

    git checkout "$PUBLISH_BRANCH"
    git merge "$MAIN_BRANCH"
    git push "$REMOTE" "$PUBLISH_BRANCH"
}

# Run only when executed, not when sourced (lets tests call the functions)
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
