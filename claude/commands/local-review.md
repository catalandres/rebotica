# /local-review

Review the current git diff with Rebotica.

1. Confirm the repository has a `.rebotica.yml` or initialize one if the user asks.
2. Run:

```sh
rbtc review
```

3. Treat the output as advisory.
4. Verify any findings against the code before acting.
5. Do not apply patches from review-only output.
