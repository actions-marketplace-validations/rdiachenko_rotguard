#!/usr/bin/env bash
# Publish a GitHub release for a tag and move the major tag to the released commit.
# Usage: scripts/release-publish.sh <tag> <dist dir> <notes file>   (reads GITHUB_SHA)
set -euo pipefail

tag=${1:?usage: release-publish.sh <tag> <dist dir> <notes file>}
dist=${2:?usage: release-publish.sh <tag> <dist dir> <notes file>}
notes=${3:?usage: release-publish.sh <tag> <dist dir> <notes file>}
sha=${GITHUB_SHA:?GITHUB_SHA is not set}

if ! [[ $tag =~ ^v([0-9]+)\.[0-9]+\.[0-9]+$ ]]; then
    echo "release-publish: tag $tag is not of the form vX.Y.Z" >&2
    exit 1
fi
major=${BASH_REMATCH[1]}
version=${tag#v}

assets=()
for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
    asset="rotguard-$version-$target"
    if ! [ -f "$dist/$asset" ]; then
        echo "release-publish: $dist lacks $asset" >&2
        exit 1
    fi
    assets+=("$dist/$asset")
done
count=$(find "$dist" -maxdepth 1 -name 'rotguard-*' | wc -l)
if [ "$count" -ne 2 ]; then
    echo "release-publish: $dist must hold exactly 2 rotguard-* files, found $count" >&2
    exit 1
fi

(cd "$dist" && sha256sum rotguard-* > SHA256SUMS)

gh release create "$tag" --draft --verify-tag --title "$tag" --notes-file "$notes"

if ! gh release upload "$tag" "${assets[@]}" "$dist/SHA256SUMS"; then
    echo "release-publish: uploading to $tag failed, deleting the draft" >&2
    gh release delete "$tag" --yes
    exit 1
fi

if ! gh release edit "$tag" --draft=false; then
    # The request may have landed with its response lost, so ask before deleting.
    state=$(gh release view "$tag" --json isDraft --jq .isDraft) || state=unknown
    case "$state" in
        true)
            echo "release-publish: publishing $tag failed, deleting the draft" >&2
            gh release delete "$tag" --yes
            exit 1
            ;;
        false)
            echo "release-publish: gh release edit failed but $tag is published, moving v$major" >&2
            ;;
        *)
            echo "release-publish: $tag may be published; nothing deleted, check gh release view $tag" >&2
            exit 1
            ;;
    esac
fi

git tag -f "v$major" "$sha"
git push -f origin "v$major"
