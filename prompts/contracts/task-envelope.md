# Task Envelope Contract

Every delegated run must include an explicit task envelope.

```yaml
task_id: local-001
mode: review | explain | propose_tests | propose_patch
goal: ""
project_context: ""
allowed_files: []
forbidden_files: []
sensitive_files: []
commands_to_run: []
max_changed_lines: 300
max_files_changed: 5
output_format: analysis | json | unified_diff
acceptance_criteria: []
risk_level: low | medium | high
```

The envelope is a boundary, not a suggestion. If the task cannot be done inside the envelope, the local model must return `REFUSAL_WITH_REASON`.
