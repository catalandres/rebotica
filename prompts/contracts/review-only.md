# Review-Only Contract

Mode: `review`

The local model may inspect supplied context and return advisory analysis.

The local model must not:

- Propose a full patch unless asked for a patch mode.
- Ask to edit files.
- Claim tests were run.
- Expand scope beyond the supplied diff and instructions.

Required return kind:

```text
ANALYSIS_ONLY
```

Required content:

- Summary.
- Concrete issues, ordered by severity.
- Missing tests.
- Risks and uncertainty.
- Recommended next step.
