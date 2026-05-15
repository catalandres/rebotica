# Self-Healing

Rebotica should improve operationally over time without giving local models broader authority.

The learning loop is:

```text
task -> model output -> review -> checks -> acceptance/rejection -> retrospective -> prompt/config/routing update
```

## Run Storage

Runs are stored privately under:

```text
~/.rebotica/runs/
```

Each run may contain:

```text
task-envelope.yml
prompt.md
skills.json
model-response.md
parsed-output.json
feedback.yml
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
rbtc retro RUN_ID
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

## Scorecards

Prime can score a model-backed run:

```sh
rbtc score RUN_ID --rating 4 --accepted --label useful-review
```

This writes run-local `feedback.yml`, appends to `~/.rebotica/model-events.jsonl`, and refreshes `~/.rebotica/model-scorecards.yml`.

## Comment Cards

Comment cards are product feedback about Rebotica, not model performance feedback:

```sh
rbtc comment-card new --from-run RUN_ID --kind ux --area review --source prime --title "..."
```

Cards remain local under `~/.rebotica/comment-cards/pending/` until Prime or a human grants submission consent and submits them.

## Model Scorecards

Model notes live at:

```text
~/.rebotica/model-scorecards.yml
```

Track what each model is good and weak at. The right long-term routing is empirical, not theoretical.
