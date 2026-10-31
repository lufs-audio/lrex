---
name: speccing
description: "Pointer stub — this repo does NOT vendor the skill. Read the canonical copy before speccing."
---

# Speccing — see the canonical copy

This repo does not vendor this skill. Read it from the single source of truth:

- `danialrami/dotfiles` → `agents/.agents/skills/speccing/SKILL.md` (canonical)
- `lufs-audio/kb` → `skills/hyperagent/speccing/SKILL.md` (mirror)

**Why this file is now a pointer.** The vendored copy that used to live here had
drifted a version behind and still taught the retired layout (one root `SPEC.md`
plus a shared `docs/units/`). That stale copy is exactly why this repo's Phase 2
spec was authored into `docs/units/` instead of the current
`docs/specs/<timestamp>_<slug>/units/`, and had to be relocated afterwards.

Per `lufs-audio/bplate` → `docs/units/08-documentation-and-workflow-standard.md`,
global skills live **once** in `dotfiles` and are never copied into scaffolded
repos — a local copy is a copy that can drift.
