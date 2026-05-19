# Net Token Savings — Method & Caveats

`rebotica`'s pitch is that delegating to a local apprentice saves Prime (Claude, Codex, etc.) tokens on the same work, at the cost of one MCP roundtrip plus the apprentice's local inference. "Net tokens saved" is a counterfactual: *how many tokens would Prime have spent doing this work itself?* That number is unobservable in production. Every measurement strategy below is a different approximation, with different fidelity/cost tradeoffs.

This document describes the **method C** estimator: post-hoc estimation from ledger data. It is shipped as the running-total surface. It is honest about its limits, and it is *not* calibrated against actual paired A/B measurements yet — that calibration is a follow-up.

## What the ledger captures today

Every `run_completed` event records (when available):

- `apprentice_prompt_tokens` — exact, from the provider's `usage` block (LM Studio reports it; some upstream proxies strip it).
- `apprentice_completion_tokens` — exact, same source.
- `envelope_bytes` — exact, computed from the post-validation parsed payload. Approximate Prime-side roundtrip cost is `envelope_bytes / 4` (tokens).

The full prompt the apprentice received is persisted to `~/.rebotica/runs/<id>/prompt.md`, so consumers who want to estimate `referenced_input_bytes` (the size of the user-supplied content embedded in the prompt) can recompute it from disk without a ledger schema change.

## The estimator (v1)

```
net_saved ≈ counterfactual_input + counterfactual_output
          - apprentice_prompt_tokens
          - apprentice_completion_tokens
          - envelope_bytes / 4
```

with these approximations:

- `counterfactual_input ≈ referenced_input_bytes / 4`. The bytes Prime would have had to read to form an equivalent opinion. Computed per-mode from the run dir's `prompt.md` minus the rebotica template header (estimate: subtract the per-mode prompt template length).
- `counterfactual_output ≈ envelope_bytes / 4`. Prime would have written something roughly the size of the apprentice's structured output. This is a *rough* equivalence — Prime often produces more verbose prose; for v1 we don't apply a multiplier.

Subtracting `envelope_bytes / 4` on the cost side and adding the same number on the counterfactual side means the envelope term cancels in the algebra:

```
net_saved ≈ referenced_input_bytes / 4
          - apprentice_prompt_tokens
          - apprentice_completion_tokens
```

Intuitively: the apprentice saves Prime from reading all those files, at the cost of the local model's token spend. The output cost approximately cancels because Prime would have written equivalent content either way.

`envelope_bytes` is still captured in the ledger even though it cancels in this formula. The reason: consumers who choose a different counterfactual model (e.g. one that applies a "Prime writes 1.5× more than the apprentice" multiplier, or one that doesn't assume the outputs are equivalent) will need it. The formula is a v1 *consumer* of the field, not its only purpose; future consumers may use it un-cancelled.

## What this estimator deliberately does not model

These are all known sources of bias. We accept them in v1 in exchange for shipping a number at all.

1. **No overhead modeling for false-positive rebuttal.** When Prime rejects an apprentice finding, it spends extra tokens explaining what was wrong. The `prime_disposition` event captures whether the run was accepted/rejected/edited-then-used, but we don't subtract a disposition-keyed overhead constant from `net_saved` in v1. This systematically *overstates* savings — runs scored `reject` were less cheap than they look.
2. **No multi-turn savings credit.** Prime doing the work itself often takes multiple tool-call turns (read file, read another file, write review, revise). Each turn re-bills the conversation history. The apprentice does it in one external shot. The estimator only credits the avoided file reads, not the avoided re-billings, so it *understates* savings for tasks that would have required multiple turns.
3. **No tokenizer correction.** We use `bytes / 4` as a generic English-text approximation. Real tokenizer ratios vary by model, content type, and language. For Anthropic models reviewing English code, the approximation is in the right ballpark; for other languages or content types, it can be off by 20–30%.
4. **No calibration against paired A/B runs.** Until we run measured A/B comparisons (method A in [the design discussion](https://github.com/catalandres/rebotica/issues/68)) and adjust the coefficients to match, the absolute number is an educated guess. Use the *trend* (how it changes over time, between models, between modes) more than the absolute level.

## Why these omissions are acceptable for v1

The ledger fields are the load-bearing artefact. Once they're being captured on every run, anyone can compute a different formula later — including one that *does* model overhead and apply tokenizer correction. The fields are additive; the formula is consumer-side; the corpus is what compounds. Getting the fields right (and labeled) matters more than getting the formula right on day one.

A consumer publishing a public number from this data should always disclose:

- Which formula was used.
- That it is uncalibrated.
- The fraction of runs with `usage` data (some providers omit it; those runs can't contribute to the apprentice-cost side).
- The disposition mix (a corpus that's 80% `reject` should produce a more skeptical headline than one that's 80% `accept`).

## Calibration plan (deferred)

When we have enough corpus to bother (~200+ scored runs across multiple modes), run ~5 paired A/B's by hand:

1. Pick a representative diff / file set.
2. Run Prime through both workflows: with and without the apprentice.
3. Capture total API token consumption for each path.
4. Compute observed `net_saved` and the v1 estimator's prediction.
5. Adjust the per-mode coefficients (template length, output multiplier) to minimize the residual.

Result: an anchored multiplier we can attach to the estimator and publish with confidence. Until then, treat the running total as directional, not nominal.

## See also

- [output-contract.md](output-contract.md#event-payload-shapes-v03) — the on-the-wire shape of the captured fields.
- Issue tracker — open issues tagged `measurement` or `instrumentation` cover the follow-ups (calibration harness, `rbtc gain --net` consumer, tokenizer correction).
