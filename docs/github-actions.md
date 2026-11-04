# Run rotguard weekly on a site

Copy this workflow to `.github/workflows/rotguard.yml` on the site. Every Monday at 06:00 UTC it runs `rotguard --apply` on the site's `main` and opens a pull request with the safe fixes. The PR body is rotguard's markdown report: what it fixed, what needs a hand fix, and a folded list of the rest. The run's summary page shows the same report. If a link needs a hand fix, the job fails after the PR step.

```yaml
name: rotguard
on:
  schedule:
    - cron: "0 6 * * 1"
  workflow_dispatch:
permissions:
  contents: write
  pull-requests: write
concurrency: rotguard
jobs:
  fix:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v7
      - id: rotguard
        uses: rdiachenko/rotguard@main
      - if: steps.rotguard.outputs.exit-code == '0' || steps.rotguard.outputs.exit-code == '1'
        uses: peter-evans/create-pull-request@v8
        with:
          branch: rotguard/fixes
          delete-branch: true
          title: Fix rotted links
          commit-message: Fix rotted links
          body-path: ${{ steps.rotguard.outputs.report }}
      - if: steps.rotguard.outputs.exit-code != '0'
        env:
          CODE: ${{ steps.rotguard.outputs.exit-code }}
        run: |
          if [ "$CODE" = 1 ]; then
            echo "::error::rotguard exited 1: a link needs a hand fix, see the Run rotguard --apply step"
          else
            echo "::error::rotguard failed (exit $CODE), see the Run rotguard --apply step"
          fi
          exit 1
  notify:
    needs: [fix]
    if: always() && contains(needs.*.result, 'failure')
    uses: ./.github/workflows/telegram-notify.yml
    with:
      title: "rotguard: weekly run failed"
    secrets: inherit
```

The job reads the action's outputs `exit-code` and `report`; the action also writes the report to the job summary. The action keeps its compiled dependencies in the site repo's Actions cache, so a week whose `Cargo.lock` and stable compiler haven't changed skips most of the build. The PR is rebuilt from `main` on every run, so an open PR is updated, not duplicated.

## One setting

In the site repo: Settings, Actions, General, Workflow permissions, "Allow GitHub Actions to create and approve pull requests". Without it the PR step fails.

## Outcomes

| rotguard exit | Workspace changed | PR step | Job | Alert |
|---|---|---|---|---|
| 0 | yes | opens or updates the PR | passes | no |
| 0 | no | closes an open PR, else nothing | passes | no |
| 1 | yes | opens or updates the PR | fails | yes |
| 1 | no | closes an open PR, else nothing | fails | yes |
| 2, or any other code | any | skipped | fails | yes |
| build or fetch fails | no | skipped | fails | yes |

On exit 2 or higher, `--apply` may have written some files and not others, so no PR opens.

## Notes

- Merging the PR changes only the sources. If your site deploys on a separate workflow, run it as usual.
- A PR opened with `GITHUB_TOKEN` may hold the site's required checks until someone with write access clicks "Approve workflows to run".
- The `notify` job calls a reusable workflow, `telegram-notify.yml`, that sends a message when the run fails. Point it at your own alert workflow, or drop the job: GitHub still emails the person who last edited the schedule when a scheduled run fails.
- Run it now with `gh workflow run rotguard.yml`.
