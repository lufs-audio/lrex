# Trigger eval — speccing

Small labeled set of prompts to confirm the skill fires when it should and
stays silent when it shouldn't. Speccing is advertised by default (no
`disable-model-invocation`), so it can auto-fire on decomposition/planning
tasks; it is also force-loaded by `end-to-end` PHASE 0.

## Should trigger

- "Break this project into spec + unit contracts before anyone starts." → invoke speccing.
- "Turn this idea into a SPEC and per-unit contract docs with acceptance criteria." → invoke.
- "Decompose this feature so a fresh agent could act on each piece with no other context." → invoke.

## Should NOT trigger

- "Quick fix: correct this off-by-one in `parity_to_token.py`." → do inline, no skill.
- "Dispatch the approved units to Herdr builders on exe.dev and land PRs." → that's `end-to-end`, not speccing; speccing already produced the contracts.
- "Read my daily note and summarize the day." → unrelated, silent.

Call: confirm speccing loads on decomposition prompts and stays silent on inline
and dispatch-only prompts; confirm `end-to-end` force-loads it during PHASE 0.
