# Validation records

| Date | Evidence |
|---|---|
| [2026-11-01 review follow-up](2026-11-01/review-followup.md) | Hardware/GTK fixes, duplicate-row safety, feature modules, and 982-test cleanup pass |
| [2026-11-01](2026-11-01/report.md) | Baseline release audit, compatibility fixes and hardware sample |
| [2026-10-31](2026-10-31/report.md) | Earlier experiments, rejected approaches and retained failures |

Use `YYYY-MM-DD/report.md` for reviewed findings and their scope. Keep supporting
measurements beside the report. Record the source/artifact, commands, environment,
results and limitations. Preserve dated observations; use release status for
current progress. A procedure or missing result is not a pass.

Raw local console output belongs in [logs](../../logs/README.md), which Git ignores.
Packages and source-bound candidate bundles stay outside the checkout.

## JSON evidence and fixtures

The dated `hardware.json`, `native.json`, `software.json`, and `gpu.json` files
record the specific runtime identified by `artifact_sha256`. They are historical
measurements, not live status for the branch head. Keep their measured values,
failed acceptance, and incomplete-protocol fields intact; a new runtime needs a
new measurement. Use [release status](../release-status.md) for current progress.

JSON under `fixtures/` is controlled parser input, not host data or release
metadata. `fixtures/journal/malformed.jsonl` deliberately mixes invalid JSON and
valid records to exercise error handling. It must not be normalized into valid
JSON during a repository-wide formatting or freshness pass.
