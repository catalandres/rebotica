# /local-patch

Ask Atelier for a bounded patch draft from a task envelope.

Expected argument:

```text
.atelier/tasks/task-envelope.yml
```

Run:

```sh
atelier patch .atelier/tasks/task-envelope.yml --dry-run
```

Review the unified diff manually. Do not apply it automatically. Check forbidden paths and run project checks after any accepted edit.
