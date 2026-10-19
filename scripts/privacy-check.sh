#!/usr/bin/env bash
# Keeps private data out of the repo and its history.
#
#   privacy-check.sh --staged     added lines in the index (pre-commit hook)
#   privacy-check.sh --msg FILE   a commit message (commit-msg hook)
#   privacy-check.sh --all        every tracked text file (CI)
#
# Flags home paths, temp/scratch paths, tokens and private keys, email
# addresses other than the commit author's or placeholders, and any string in
# the untracked per-clone denylist at $(git rev-parse --git-common-dir)/info/privacy-denylist
# (one string per line, case-insensitive; keeps personal strings out of the repo
# itself). Prints locations and kinds only, never the matched text, so CI logs
# can't leak what it found.
set -euo pipefail

self="scripts/privacy-check.sh"
root=$(git rev-parse --show-toplevel)
denylist="$(git rev-parse --path-format=absolute --git-common-dir)/info/privacy-denylist"
author=$(git config user.email || true)

email_re='[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}'
allowed_email_re='@users\.noreply\.github\.com$|@example\.(com|org|net)$|^git@github\.com$'
if [ -n "$author" ]; then
  allowed_email_re="$allowed_email_re|^$(printf '%s' "$author" | sed 's/[.[\*^$+?(){}|]/\\&/g')$"
fi

patterns='home path|/(Users|home)/[A-Za-z0-9._-]+
temp path|/private/(tmp|var/folders)/|/var/folders/[A-Za-z0-9_]{2}/
GitHub token|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}
Anthropic key|sk-ant-[A-Za-z0-9_-]{10,}
API key|sk-[A-Za-z0-9]{32,}
Slack token|xox[abpr]-[A-Za-z0-9-]{10,}
AWS key|AKIA[0-9A-Z]{16}
private key|-----BEGIN [A-Z ]*PRIVATE KEY-----'

records=$(mktemp)
hits=$(mktemp)
trap 'rm -f "$records" "$hits"' EXIT

# Emit "path<TAB>line<TAB>text" records to scan.
case "${1:-}" in
  --staged)
    git diff --cached -U0 --no-color --diff-filter=ACMR | awk '
      /^\+\+\+ /    { path = substr($0, 7); next }
      /^@@/         { match($0, /\+[0-9]+/); n = substr($0, RSTART + 1, RLENGTH - 1) + 0; next }
      /^\+/         { print path "\t" n "\t" substr($0, 2); n++ }
    ' > "$records" ;;
  --msg)
    awk '!/^#/ { print "commit message\t" NR "\t" $0 }' "${2:?commit message file required}" > "$records" ;;
  --all)
    (cd "$root" && git ls-files -z | xargs -0 grep -Il --null '' 2>/dev/null \
      | xargs -0 awk '{ print FILENAME "\t" FNR "\t" $0 }') > "$records" || true ;;
  *)
    echo "usage: $0 --staged | --msg FILE | --all" >&2; exit 64 ;;
esac

scan=$(mktemp)
grep -v "^$self	" "$records" > "$scan" || true
mv "$scan" "$records"

flag() { # kind; reads matching records on stdin
  while IFS="$(printf '\t')" read -r path line _; do
    printf '%s:%s: %s\n' "$path" "$line" "$1" >> "$hits"
  done
}

printf '%s\n' "$patterns" | while IFS='|' read -r kind regex; do
  grep -E -e "$regex" "$records" | flag "$kind" || true
done

grep -E "$email_re" "$records" | while IFS= read -r rec; do
  text=${rec#*	}; text=${text#*	}
  if printf '%s\n' "$text" | grep -oE "$email_re" | grep -qvE "$allowed_email_re"; then
    printf '%s\n' "$rec" | flag "email address"
  fi
done || true

if [ -s "$denylist" ]; then
  terms=$(mktemp)
  sed -e '/^#/d' -e '/^[[:space:]]*$/d' "$denylist" > "$terms"
  if [ -s "$terms" ]; then
    grep -iF -f "$terms" "$records" | flag "denylisted string" || true
  fi
  rm -f "$terms"
fi

if [ -s "$hits" ]; then
  sort -u "$hits" >&2
  echo "privacy-check: blocked $(sort -u "$hits" | wc -l | tr -d ' ') finding(s). Remove the data, or fix a false positive in $self." >&2
  exit 1
fi
