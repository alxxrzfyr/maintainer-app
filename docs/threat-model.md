# Maintainer Helper Threat Model

## Overview

The Maintainer privileged helper (`maintainer-helper`) runs as root via D-Bus activation and executes a fixed set of system operations on behalf of the unprivileged GUI. This document identifies threats against the helper and maps each to a mitigation or explicit non-goal.

## Assets

| Asset | Value |
|---|---|
| Process table integrity | Arbitrary signal delivery could kill critical services |
| Package cache | Corruption or unauthorized clearing disrupts updates |
| Journal data | Unauthorized vacuum destroys diagnostic evidence |
| SMART subsystem | Unauthorized refresh could mask disk degradation timing |
| Root execution context | Full system compromise if helper is subverted |

## Trust Boundaries

```text
Unprivileged GUI process (UID 1000)
  │
  │  D-Bus system bus (message routing, credential lookup)
  │
Privileged helper (UID 0, D-Bus activated)
  │
  │  Polkit authority (interactive or cached authorization)
  │
Linux kernel / system services
```

## Threats and Mitigations

### T1: Caller Spoofing

**Threat:** A malicious process impersonates the legitimate GUI caller by crafting D-Bus messages with a fake sender name.

**Mitigation:** Resolve caller UID and PID through the D-Bus daemon's `GetConnectionUnixProcessID` and `GetConnectionUnixUser` methods, which use kernel credentials (SO_PEERCRED). Never trust self-reported identity from the message payload.

**Test:** `test_unknown_caller_rejected`, `test_wrong_uid_rejected`

### T2: PID Reuse Race

**Threat:** Between authorization check and signal delivery, the target process exits and its PID is reassigned to a different process. The helper then kills an unrelated process.

**Mitigation:** Read `/proc/<pid>/stat` start time (field 22, `starttime`) during request validation. Revalidate start time immediately before `kill(2)`. Reject if start time differs.

**Test:** `test_pid_reuse_detected`

### T3: Forbidden Signal Injection

**Threat:** Caller requests SIGKILL, SIGSTOP, or other signals beyond the sanctioned SIGTERM to cause data loss or denial of service.

**Mitigation:** The `TerminateProcessRequest` DTO does not accept a signal field. The helper always sends SIGTERM only. No method exposes signal selection.

**Test:** `test_no_signal_field_in_dto`

### T4: Path Substitution / Symlink Attack

**Threat:** Caller passes a crafted path (e.g., `/var/cache/apt/../../etc/shadow`) or a symlink to redirect file operations to sensitive locations.

**Mitigation:** Package cache clean uses a fixed `apt-get clean` invocation with no path parameters from the caller. No operation accepts arbitrary filesystem paths. Journal vacuum passes only validated numeric size/age parameters to `journalctl`. SMART refresh targets a UDisks2 D-Bus object path validated against `/org/freedesktop/UDisks2/drives/` prefix.

**Test:** `test_path_injection_rejected`

### T5: Argument Injection via Shell

**Threat:** If the helper constructs shell command strings from caller input, metacharacters enable arbitrary command execution.

**Mitigation:** No shell is ever invoked. All external programs are called via `Command::new()` with explicit argument arrays. Parameters that reach command arguments are validated against strict patterns (digits-only for sizes, alphanumeric with underscores for device identifiers).

**Test:** Structural — no `shell=true`, no string concatenation into commands.

### T6: Authorization Replay / Stale Authorization

**Threat:** A cached Polkit authorization token from a legitimate session is replayed after the user logs out or the session changes.

**Mitigation:** Each D-Bus method call triggers a fresh Polkit `CheckAuthorization` with the caller's current subject (system-bus-name). Polkit session lifecycle handles token expiry. The helper does not cache authorization results.

**Test:** `test_polkit_denial_rejected`

### T7: Confused Deputy

**Threat:** The helper performs a privileged action that benefits an attacker because it trusts the wrong identity or grants the wrong action.

**Mitigation:** Polkit action IDs are specific per operation (`io.github.maintainerapp.terminate-process`, `io.github.maintainerapp.clean-package-cache`, etc.). Each method checks its own specific action. A user authorized for package cache cannot terminate processes.

**Test:** `test_wrong_action_id_rejected`

### T8: Denial of Service via Request Flooding

**Threat:** A caller floods the helper with concurrent requests, consuming resources and blocking legitimate operations.

**Mitigation:** The helper enforces a concurrent request limit (default: 4). Requests beyond the limit are rejected immediately with a "busy" error. The helper exits after an idle timeout (default: 60 seconds), preventing resource accumulation.

**Test:** `test_concurrent_limit_enforced`

### T9: Oversized / Malformed Messages

**Threat:** Crafted D-Bus messages with excessively large payloads or malformed structure cause memory exhaustion or parsing panics.

**Mitigation:** D-Bus has a built-in 128 MiB message size limit. The helper's DTO deserialization uses `zbus` with serde which rejects unknown fields and validates types. All numeric parameters have explicit bounds checks in the validation layer.

**Test:** `test_oversized_values_rejected`, `test_malformed_dto_rejected`

### T10: Journal Vacuum Abuse

**Threat:** Caller passes extreme values (zero bytes, zero seconds) to `journalctl --vacuum-size` or `--vacuum-time`, destroying all journal history.

**Mitigation:** Validate that vacuum size is between 100 MiB and 4 GiB, and vacuum age is between 1 day and 365 days. Reject values outside these bounds.

**Test:** `test_journal_bounds_enforced`

### T11: Partial Execution and Inconsistent State

**Threat:** The helper crashes mid-operation, leaving the system in an inconsistent state.

**Mitigation:** Each operation is atomic from the caller's perspective. `apt-get clean` and `journalctl --vacuum-*` are inherently atomic. Process termination is a single syscall. SMART refresh is a single D-Bus call. No operation has multi-step commit semantics that could leave partial state.

**Non-goal:** Rollback of completed operations (apt-get clean cannot be un-cleaned).

### T12: Audit Data Leakage

**Threat:** Error messages or logs expose sensitive system information (paths, PIDs of other users' processes, etc.) to the unprivileged caller.

**Mitigation:** Error responses contain only generic status codes and safe descriptions. Internal details are logged to the journal (readable only by root or journal group) but not returned over D-Bus.

## Non-Goals

- Protecting against a caller that already has root (they do not need the helper).
- Rate-limiting by user identity (single-user desktop assumption).
- Protecting against kernel-level attacks on D-Bus credential reporting.
- Network-facing attack surface (helper has no network access via systemd sandboxing).

## Systemd Hardening

The helper unit applies:
- `NoNewPrivileges=yes` (except for required capability)
- `PrivateTmp=yes`
- `ProtectSystem=strict`
- `ProtectHome=yes`
- `RestrictNamespaces=yes`
- `RestrictRealtime=yes`
- `MemoryMax=128M`
- `CPUQuota=25%`
- `IPAddressDeny=any`
- `CapabilityBoundingSet=CAP_KILL CAP_DAC_READ_SEARCH`
- `SystemCallFilter=@system-service`
