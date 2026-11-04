# Security

Report a vulnerability privately through
[GitHub's advisory form](https://github.com/rdiachenko/rotguard/security/advisories/new), not in a
public issue. I'll reply within a week.

rotguard rewrites files with `--apply` and often runs in CI with a token that can push, so these
matter most: a write outside the configured content files, a fix that injects text other than the
new URL, and anything a linked site's response can do beyond changing a verdict.

Only the latest release gets fixes.
