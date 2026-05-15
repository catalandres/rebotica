# /local-tests

Ask Rebotica for missing test proposals for selected files.

Expected argument:

```text
path/to/file [path/to/another-file]
```

Run:

```sh
rbtc run tests path/to/file
```

The local model should propose tests, not edit files. Prime decides whether to implement them.
