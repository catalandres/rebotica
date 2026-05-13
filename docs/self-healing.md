# Self-Healing

Atelier should improve operationally over time without giving local models broader authority.

The learning loop is:

```text
task -> worker output -> review -> checks -> acceptance/rejection -> retrospective -> prompt/config/routing update
```

## Run Storage

Runs are stored privately under:

```text
~/.atelier/runs/
```

Each run may contain:

```text
task-envelope.yml
prompt.md
model-response.md
parsed-output.json
applied.patch
review.md
test-output.log
scorecard.yml
retrospective.md
```

Do not store secrets or huge files.

## Retrospective

Create a retrospective:

```sh
atelier retro RUN_ID
```

Ask:

- What failed?
- What surprised us?
- Was context missing?
- Was the task too broad?
- Did the local model violate scope?
- Did checks catch the issue?
- Should project config change?
- Should prompt or model routing change?

## Model Scorecards

Model notes live at:

```text
~/.atelier/model-scorecards.yml
```

Track what each model is good and weak at. The right long-term routing is empirical, not theoretical.
