# Unit-of-work contracts

The unit contract is the *only* thing a builder sees. The single highest-leverage
practice is making it self-contained: a fresh agent with **only this document**
plus the repo should be able to do the unit and know when it's done. If it can't,
the contract is too weak.

## Template (`docs/units/NN-slug.md`)

```markdown
# Unit NN — <slug>

## Objective
<One sentence: what "done" means for this unit alone.>

## Context
<Pointers: relevant files, the branch, docs worth reading. Do NOT inline large
explanations here — point, don't paste.>

## Acceptance criteria
- [ ] <testable criterion 1 — maps to a check>
- [ ] <criterion 2>
- ...

## Interface contract
<What this unit must honor that others depend on: function/signature/route/
schema/DB-name. This is where units touch each other — keep it explicit.>

## Boundaries — do NOT touch
<Files, dirs, or other units' surface. Prevents collisions and scope creep.>

## Output
<What a successful PR looks like: set of files, tests, commit-message style,
where tests live.>

## Verification
<Exact commands that prove it (build, targeted tests). The PR must be assessable
against ONLY this doc.>
```

## Grading a contract before you dispatch it

- Can a builder act on it with **no other context**? If no, fix it before dispatch.
- Are acceptance criteria **testable** (each answerable from a check), not vibes?
- Is the interface contract explicit enough that two builders would write
  compatible glue?
- Are boundaries listed so two parallel units won't both claim the same file?

## Splitting a spec into units

- Split on **surface/discipline**, not on file count: frontend vs backend vs
  schema vs tooling, or independent feature slices.
- Two units that will both edit `service/x.py` are a design problem — either
  merge them or draw a hard boundary (one owns it, the other only reads it).
- The split is a *plan* artifact; when a unit turns out too big, sub-split it
  before anyone starts on it, not during.
