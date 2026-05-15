# Providers

Rebotica talks to OpenAI-compatible chat completion endpoints. LM Studio is the default local provider, but the config is intentionally provider-oriented so teams can route work to other compatible endpoints.

This is a useful abstraction. The constraint is that providers should remain narrow: base URL, optional auth, and model routing. Avoid adding provider-specific behavior until a real workflow needs it.

## Configuration

```yaml
providers:
  default: lmstudio
  lmstudio:
    kind: openai-compatible
    base_url: http://127.0.0.1:1234/v1
  openai:
    kind: openai-compatible
    base_url: https://api.openai.com/v1
    api_key_env: OPENAI_API_KEY

models:
  default: local-coder
  review: local-coder
  explain: local-coder
  tests: local-coder
  patch: local-coder
  aliases:
    local-coder: huihui-qwen3.6-35b-a3b-claude-4.7-opus-abliterated-mlx
```

## Provider Selection

Use config defaults:

```sh
rbtc health
rbtc run review
```

Select a provider:

```sh
rbtc health --provider openai
REBOTICA_PROVIDER=openai rbtc run review
```

Override the URL directly:

```sh
rbtc health --base-url http://127.0.0.1:1234/v1
REBOTICA_BASE_URL=http://127.0.0.1:1234/v1 rbtc health
```

## Auth

Use environment variables for secrets:

```yaml
providers:
  openai:
    kind: openai-compatible
    base_url: https://api.openai.com/v1
    api_key_env: OPENAI_API_KEY
```

By default, Rebotica sends:

```text
Authorization: Bearer $OPENAI_API_KEY
```

Override the header or prefix only when a compatible provider requires it:

```yaml
providers:
  custom:
    kind: openai-compatible
    base_url: https://example.com/v1
    api_key_env: CUSTOM_API_KEY
    api_key_header: X-API-Key
    api_key_prefix: ""
```

Do not put API keys in `.rebotica.yml`.

## Model Routing

Model aliases keep long model ids out of commands and task envelopes:

```yaml
models:
  default: local-model
  review: reviewer
  tests: test-writer
  patch: patcher
  aliases:
    local-model: actual-model-id
    reviewer: actual-review-model-id
```

Use a raw model id or alias:

```sh
rbtc smoke --model local-model
REBOTICA_MODEL=local-model rbtc run review
```

## Route Setup

`rbtc init` creates the config and leaves model routes empty so onboarding works offline and never mutates provider settings unexpectedly. Configure routes through an explicit follow-up command:

```sh
rbtc models configure --model ACTUAL_MODEL_ID --alias local-model
```

That writes `models.aliases.local-model` and fills only empty `default`, `review`, `explain`, `tests`, and `patch` routes. Existing routes are left alone unless you pass `--force`.

When LM Studio or another OpenAI-compatible provider is reachable, you can opt into detection:

```sh
rbtc models configure --detect
```

Detection writes routes only when the provider returns exactly one model. If the provider is unavailable, returns no models, or returns multiple candidates, Rebotica prints the next command to run and leaves `.rebotica.yml` unchanged.

## Design Boundary

Rebotica should not become a provider SDK. The first contract is:

- OpenAI-compatible `/models`.
- OpenAI-compatible `/chat/completions`.
- Optional bearer-style auth.
- Project-level routing and aliases.

If a provider needs different request or response semantics, add it only after capturing the workflow and safety implications.
