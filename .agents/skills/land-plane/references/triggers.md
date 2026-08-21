# land-plane — trigger evaluation

Confirms the `land-plane` description fires on merge/finalization tasks and stays
silent on adjacent-but-different work.

## Should fire (trigger)

- "We're ready to ship — verify, open the PRs, merge if they're clean, then clean up the branches"
- "Land this feature / finalize this branch for merge"
- "Open PR(s) for these branches, check they're mergeable, and merge them"
- "Wrap up the batch: final checks, merge, confirm, delete the old branches"
- "Take this docs/ work through to merged on main and clean up"

## Should NOT fire (stays silent)

- "Review this PR / give feedback on this diff" (that's review, not landing)
- "There's a merge conflict, help me resolve it" (conflict resolution, not the
  clean-then-merge sequence)
- "Make a one-off commit and push it" (no PR, no multi-step landing)
- "Open a branch to start a feature" (starting work, not finalizing it)
- "Explain how GitHub PRs work" (conceptual, not the workflow)

## Notes

- Description is imperative + trigger-first ("Finalize work for merge"), names
  the concrete steps, and carries a boundary ("Not for reviewing... not for
  one-off commits without PRs") to avoid over-triggering on review or
  conflict-resolution requests.
