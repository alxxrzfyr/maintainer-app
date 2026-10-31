# Local logs

Store raw output in `logs/YYYY-MM-DD/<run>/`:

```text
logs/2026-11-01/release-audit/
  build.log
  tests.log
  ci.log
  performance.log
```

Git ignores every entry here except this guide. Raw logs may contain local paths
or diagnostic data. Never force-add or upload them. Publish only reviewed,
redacted summaries under `docs/validation/YYYY-MM-DD/`.

Keep `.deb` packages and candidate bundles outside the project.
