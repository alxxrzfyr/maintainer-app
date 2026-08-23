# Spec: Polkit Policy Consistency & Packaging Fix

**Date:** 2026-08-23
**Scope:** Security boundary — packaging, helper API, regression tests

---

## Problem

1. **Stale-policy failure:** `packaging/debian/postinst` does not reload Polkit after installing the policy file. Users who install/upgrade without rebooting get silent authorization denial on every privileged action.

2. **No consistency guard:** The six Polkit action IDs defined in `data/polkit/io.github.maintainerapp.policy` are not validated against the code constants in `crates/maintainer-helper/src/api.rs::actions`. Drift is undetectable until runtime.

## Requirements

- R1: After `.deb` install/upgrade, `polkitd` must recognize the new policy immediately (no reboot required).
- R2: A test must fail if the set of action IDs in the XML policy file diverges from the set known to the codebase.
- R3: A test must fail if `postinst` loses the Polkit reload command.
- R4: Add `INSTALL_UPDATES` constant to `api::actions` so all six actions have typed references.

## Constraints

- No changes to the Polkit policy XML itself (actions are correct).
- No changes to the helper's authorization logic.
- The `postinst` reload must be conditional on Polkit actually running (guard with `is-active`), and must not fail the install if Polkit is absent.
- Tests use `CARGO_MANIFEST_DIR` relative paths for portability across workspaces.

## Deliverables

| File | Change |
|---|---|
| `crates/maintainer-helper/src/api.rs` | Add `INSTALL_UPDATES` constant |
| `crates/maintainer-helper/tests/policy_consistency.rs` | New: parse XML, assert action IDs match code, assert exec-path annotation |
| `crates/maintainer-app/tests/packaging_postinst.rs` | New: assert `postinst` contains Polkit reload |
| `packaging/debian/postinst` | Add guarded `systemctl restart polkit.service` |

## Verification

```bash
cargo test -p maintainer-helper policy_xml_action_ids_match_code_constants -- --exact
cargo test -p maintainer-app postinst_reloads_polkit_after_policy_install -- --exact
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
