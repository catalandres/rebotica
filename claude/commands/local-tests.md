# /local-tests

Ask Atelier for missing test proposals for selected files.

Expected argument:

```text
path/to/file [path/to/another-file]
```

Run:

```sh
atelier tests path/to/file
```

The local worker should propose tests, not edit files. The root coordinator decides whether to implement them.
