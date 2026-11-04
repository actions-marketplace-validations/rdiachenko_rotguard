#!/usr/bin/env bash
# Verify that the pushed tag matches Cargo.toml and extract the release notes.
# Usage: scripts/release-verify.sh <notes file>   (run from the repo root, reads GITHUB_REF_NAME)
set -euo pipefail

notes_file=${1:?usage: release-verify.sh <notes file>}
tag=${GITHUB_REF_NAME:?GITHUB_REF_NAME is not set}

version=$(awk '
    /^\[/ { in_package = ($0 == "[package]"); next }
    in_package && /^version[[:space:]]*=/ {
        split($0, parts, "\"")
        print parts[2]
        exit
    }
' Cargo.toml)

if [ "$tag" != "v$version" ]; then
    echo "release-verify: tag $tag does not match Cargo.toml version $version" >&2
    exit 1
fi

notes=$(awk -v heading="## [$version]" '
    /^## / {
        if (found) exit
        if (index($0, heading) == 1) found = 1
        next
    }
    found { print }
    END { exit !found }
' CHANGELOG.md) || {
    echo "release-verify: CHANGELOG.md has no ## [$version] section" >&2
    exit 1
}

# Trim leading and trailing blank lines.
notes=$(printf '%s\n' "$notes" | awk '
    NF { started = 1 }
    started { lines[++n] = $0 }
    END {
        while (n > 0 && lines[n] !~ /[^[:space:]]/) n--
        for (i = 1; i <= n; i++) print lines[i]
    }
')

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
printf '%s\n' "$notes" > "$tmp"
mv "$tmp" "$notes_file"
