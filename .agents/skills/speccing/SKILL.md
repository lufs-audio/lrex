---
name: speccing
description: "Turn an idea into a written specification and unit-of-work contracts: produce SPEC.md plus one docs/units/NN-slug.md contract per unit, each with objective, context, testable acceptance criteria, an explicit interface contract, boundaries, and verification. Use when a task or project should be decomposed into self-contained, independently dispatchable, reviewable units. Not for single small tasks you can do inline."
---

# Speccing

Turn an ambiguous idea into a written contract that a fresh agent can act on
with **no other context**. This is the front-end of any spec-driven build: it
produces the `SPEC.md` and the per-unit contract docs that later dispatch,
review, and integration are all judged against.

This skill deliberately stops at **present for approval**. Authorizing the work
and dispatching it are the *caller's* job — for an end-to-end build those live
in the `end-to-end` skill, but speccing is useful on its own for any job you
want decomposed before anyone writes code.

## Output — the contract boundary

At the end you produce two kinds of artifacts under the project root:

- **`SPEC.md`** — the project spec: problem, goals/constraints, design
  approach, boundaries, and overall done-criteria.
- **`docs/units/NN-slug.md`** — **one contract per unit of work**. Each must be
  self-contained so a fresh agent with only this document (plus the repo) can
  do the unit and know when it's done. If it can't, the contract is too weak.

These docs are the interface everything downstream consumes. Keep the *shape*
stable — the template in the reference is the contract's schema.

## Process

1. **Intake** — capture the idea with the user: the goal, constraints, and what
   "done" means. Ask until the boundary of "done" is crisp; don't spec on vibes.
2. **Write `SPEC.md`** — the stable project-level contract.
3. **Decompose into units** — split the work into self-contained, roughly
   disjoint units (see *Splitting a spec into units* in the reference).
4. **Write one `docs/units/NN-slug.md` per unit** using the template, then
   **grade each contract** against the checklist before you call it done (see
   *Grading a contract*).
5. **Present for approval** — surface `SPEC.md` + the unit contracts + your
   decomposition rationale. The caller decides whether to proceed. Do **not**
   dispatch or implement on your own.

## Reference

See `references/contract-template.md` for the unit contract template, the
grading checklist, and how to split a spec into units.

## What to optimize for

- Every contract self-contained — objective, acceptance criteria, interface
  surface, boundaries, verification. No tribal context.
- Acceptance criteria **testable** (each answerable from a check), not vibes.
- Interfaces explicit enough that two independent agents would write compatible
  glue; boundaries explicit enough that two parallel units won't claim the same
  file.
- **Point, don't paste** — `Context` points at files/docs rather than inlining
  big explanations.

## Provenance

Vendored from `danialrami/dotfiles` `agents/.agents/skills/speccing/` into this
repo's local `.agents/skills/`, matching `lufs-audio/bplate`'s own convention
of keeping the skills a repo's agents actually use self-contained and
discoverable without leaving the repo. dotfiles remains the canonical source —
if the two drift, dotfiles wins; re-vendor rather than diverging.
