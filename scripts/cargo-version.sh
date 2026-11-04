#!/usr/bin/env bash
# Print the version from the [package] table of a Cargo.toml.
# Usage: scripts/cargo-version.sh <Cargo.toml>
set -euo pipefail

manifest=${1:?usage: cargo-version.sh <Cargo.toml>}

if ! [ -f "$manifest" ]; then
    echo "cargo-version: $manifest does not exist" >&2
    exit 1
fi

version=$(awk '
    /^\[/ { in_package = ($0 == "[package]"); next }
    in_package && /^version[[:space:]]*=/ {
        split($0, parts, "\"")
        print parts[2]
        exit
    }
' "$manifest")

if [ -z "$version" ]; then
    echo "cargo-version: $manifest has no [package] version" >&2
    exit 1
fi
printf '%s\n' "$version"
