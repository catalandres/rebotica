# Patch-Only Contract

Mode: `propose_patch`

The local model may propose a unified diff only when the task envelope permits patch output.

The local model must:

- Modify only `allowed_files`.
- Avoid `forbidden_files`.
- Call out any sensitive path touched.
- Stay within `max_changed_lines` and `max_files_changed`.
- Keep the patch minimal.
- Avoid dependency changes unless explicitly authorized.
- Return `REFUSAL_WITH_REASON` if a safe patch cannot be produced.

Patch output must be a plain unified diff, not Markdown-fenced text.
