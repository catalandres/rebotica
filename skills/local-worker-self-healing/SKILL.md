# Local Worker Self-Healing

Use this skill after Atelier local-worker runs when the root coordinator wants to improve prompts, task envelopes, project config, model routing, or scoring rules based on observed outcomes.

This skill improves the harness. It does not grant local workers broader authority.

## Inputs To Inspect

Look under:

```text
~/.atelier/runs/RUN_ID/
```

Review:

- `task-envelope.yml`
- `prompt.md`
- `model-response.md`
- `parsed-output.json`
- `review.md`, if present
- `test-output.log`, if present
- `scorecard.yml`
- `retrospective.md`, if present

## Questions

- What failed?
- What surprised us?
- Was context missing?
- Was the task too broad?
- Did the worker violate scope?
- Did checks catch the issue?
- Should project config change?
- Should a prompt change?
- Should model routing change?
- Should the task envelope template change?

## Allowed Proposals

You may propose updates to:

- prompts
- task envelope templates
- scoring rubrics
- project config
- model selection notes
- forbidden path rules
- context packing rules

## Restricted Changes

Do not automatically modify core scripts or guard logic without root review. Propose the change first, explain the evidence, then let the root coordinator decide.
