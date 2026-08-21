---
name: land-plane
description: >
  Finalize work for merge: run the full verification suite, open PR(s), check
  they're clean and mergeable, merge in dependency order, confirm the merge,
  then clean up merged branches. Use when wrapping up a feature or batch of
  work intended to land. Not for reviewing diffs or resolving conflicts; not
  for one-off commits without PRs.
---

# Land the plane

The repeatable "ship it" sequence: verify across the board, open PR(s), check
clean, merge, confirm, clean up. Run these in order and stop at the first
failure unless the fix is trivially safe.

## Before you start

- You are on a clean, up-to-date branch you have push access to, with a real
  feature branch (or two) whose work you're about to merge into a base branch
  (typically `main`).
- Confirm the remote and your fork/upstream are set:
  `git remote -v`
- If any verification step fails, **stop and hand back to the author** with the
  failure output. Do not merge a red build.

## 1. Verify across the board

Run the project's own verification first (tests, linters, type checker, build):

```bash
# project-specific; adapt to the repo (check package.json / pyproject.toml /
# Makefile / docs/ for the canonical command)
./test.sh 2>/dev/null || pytest -q 2>/dev/null || go test ./... 2>/dev/null || make test
```

Also sanity-check state is clean and current:

```bash
git fetch origin
git status -sb                       # clean working tree?
git log --oneline -1 origin/main     # base is current?
git merge-base --is-ancestor origin/main HEAD && echo "main is ancestor of HEAD"
```

## 2. Verify branch topology (CRITICAL for stacked branches)

Before opening PRs, map the relationship between your branches:

```bash
# For each of your branches B in landing order:
git merge-base --is-ancestor origin/main origin/B && \
  echo "B is a descendant of main" || echo "B has diverged or main not fetched"

# Is A an ancestor of B? (i.e. "is B stacked on A"?)
git merge-base --is-ancestor origin/A origin/B && echo "A is ancestor of B"
```

If branch **B contains branch A**, you must land **A first**, then B — otherwise
B's PR shows A's diff too. Open PRs in dependency order (earliest ancestor
first). If a branch is *not* a descendant of `main`'s tip, decide whether to
rebase before opening PRs.

## 3. Open PR(s)

Open one PR per branch against the intended base. Include the **why** and, for
docs/ops changes, a **verification** section listing exactly what was run:

```bash
gh pr create --base main --head <branch> --title "..." --body "$(cat <<'EOF'
## What & why
...

## Verification
- [ ] test suite
- [ ] manual/live check
EOF
)"
```

Capture the PR number from the URL it prints.

## 4. Check it's clean — then merge

For each PR, in dependency order:

```bash
gh pr view <n> --json mergeable,mergeStateStatus,state,additions,deletions,changedFiles
gh pr checks <n>          # wait for these to go green
```

- `mergeable: MERGEABLE` and `mergeStateStatus: CLEAN` → good to merge.
- Merge **in dependency order**: if `docs` contains `fix`, merge `fix` first, then
  re-check the `docs` PR — its diff vs the new `main` should now be only the
  doc delta, which is the confirmation the stacking is clean.
- Merge with an explicit strategy. Prefer a merge commit for stacked/nonlinear
  history, rebase for linear:

  ```bash
  gh pr merge <n> --merge     # merge commit
  gh pr merge <n> --squash    # or squash if the project squashes
  ```

- After merging, confirm success:
  ```bash
  gh pr view <n> --json state,mergedAt   # expect MERGED
  ```

## 5. Clean up merged branches

After all PRs land, delete local + remote branches:

```bash
git checkout <base> && git pull --ff-only origin <base>
git branch -D <feature-branch>           # if it still exists locally
git push origin --delete <feature-branch>  # remote
git remote prune origin                  # drop gone remotes
```

Before deleting a branch, confirm it's safe:

```bash
git branch --merged <base> | grep -q <branch> && echo "merged, safe to delete"
```

## Notes

- The definition of "clean" is project-specific: some require `MERGEABLE` only;
  others gate on checks or even merge. Honor the repo's rules (branch
  protection, required checks) — if the UI would block it, so should you.
- When multiple PRs are stacked, do **not** open all of them at once and merge
  blindly — the diffs are only correct relative to each other in order. Open →
  verify → merge one at a time.
- If something goes wrong mid-sequence (e.g. a merge conflicts after an earlier
  one landed), stop and resolve rather than force-pushing around it. Prefer
  rebasing the un-merged branch onto the new base.

## Provenance

Vendored from `danialrami/dotfiles` `agents/.agents/skills/land-plane/` into
this repo's local `.agents/skills/`, matching `lufs-audio/bplate`'s own
convention. dotfiles remains the canonical source — re-vendor rather than
diverging if the two drift.

## This repo's specifics

- No branch protection / required status checks on `lufs-audio` (Free org
  plan) — "clean" here means GitHub's `mergeable_state == "clean"` (no
  conflicts), checked via the GitHub MCP's `pull_request_read`, not a
  `gh pr checks` gate. `unstable`/`unknown` mean "ask again shortly" (CI is
  still computing), not "failed."
- This repo squash-merges (`merge_method: "squash"`), matching the rest of the
  LUFS fleet's convention (see `ci-cd-authoring` / `kb-doc-suite`).
- CI runs on the self-hosted `lufs` fleet (`.github/workflows/ci.yml`); the
  agent's own local `cargo test`/`clippy --all-targets --all-features`/
  `fmt --all --check` reproduction under `RUSTFLAGS="-D warnings"` is the
  practical substitute for waiting on fleet CI when speed matters, but always
  still check `mergeable_state` before merging — don't skip that step.
