# Maintainer Helper Threat Model

## Overview

The Maintainer privileged helper (`maintainer-helper`) runs as root via D-Bus activation for maintenance operations and through a Polkit-described `pkexec` entrypoint for system updates. Both entrypoints expose fixed operation types rather than arbitrary commands. This document identifies threats against the helper and maps each to a mitigation or explicit non-goal.

## Assets

| Asset | Value |
|---|---|
| Process table integrity | Arbitrary signal delivery could kill critical services |
| Package cache and database | Corruption or unauthorized mutation disrupts updates or the installed system |
| Journal data | Unauthorized vacuum destroys diagnostic evidence |
| SMART subsystem | Unauthorized refresh could mask disk degradation timing |
| Root execution context | Full system compromise if helper is subverted |

## Trust Boundaries

```text
Unprivileged GUI process (UID 1000)
  │
  ├─ D-Bus system bus ─ Polkit ─ D-Bus-activated helper operations
  │
  └─ pkexec ─ install-updates Polkit action ─ validated helper CLI mode
                                              │
                                              └─ exec /usr/bin/apt-get

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

**Mitigation:** No shell is ever invoked. All external programs are called with explicit argument arrays. The update CLI accepts only `full` or `selected`; selected entries must be bounded `name=version` values with strict Debian-safe character sets. A `--` option terminator precedes every selected package. The helper then replaces itself with the fixed `/usr/bin/apt-get` executable, so no privileged child is orphaned.

**Test:** Structural — no `shell=true`, no string concatenation into commands.

### T6: Authorization Replay / Stale Authorization

**Threat:** A cached Polkit authorization token from a legitimate session is replayed after the user logs out or the session changes.

**Mitigation:** Each D-Bus method call triggers a fresh Polkit `CheckAuthorization` with the caller's current subject (system-bus-name). The helper caches no authorization results.

The policy also declines to retain them. Irreversible actions (terminating a process, vacuuming the journal, removing a kernel, and installing system updates) use `auth_admin` rather than `auth_admin_keep`, so the password is requested on every attempt and an authorization obtained for one action cannot carry a later one. Reversible or read-only actions (cleaning the APT cache, reading SMART data) resolve to `yes` for an active local session and are not prompted at all, matching `org.freedesktop.udisks2.ata-smart-update`. The update action is bound by `org.freedesktop.policykit.exec.path` to `/usr/libexec/maintainer-helper`; under `PKEXEC_UID`, every helper mode except the validated update operation is rejected.

**Test:** `test_polkit_denial_rejected`

### T7: Confused Deputy

**Threat:** The helper performs a privileged action that benefits an attacker because it trusts the wrong identity or grants the wrong action.

**Mitigation:** Polkit action IDs are specific per operation (`io.github.maintainerapp.terminate-process`, `io.github.maintainerapp.clean-package-cache`, `io.github.maintainerapp.install-updates`, etc.). Each D-Bus method checks its own action, and the update action maps only to the helper executable whose pkexec mode accepts one fixed operation. A user authorized for package cache cannot terminate processes or install updates.

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

**Mitigation:** Single-step helper operations remain atomic from the caller's perspective. System updates are intentionally different: APT/dpkg transactions are multi-step and cannot be rolled back by Maintainer. The authenticated helper uses `exec` so APT replaces the helper process, preserves progress pipes, and cannot be orphaned as a hidden privileged child. APT and dpkg provide their normal lock, journal, and recovery behavior.

**Non-goal:** Rollback of completed package changes. As with every APT frontend, interruption by cancellation, process failure, or power loss may require the administrator to run dpkg/APT recovery before another transaction.

### T12: Audit Data Leakage

**Threat:** Error messages or logs expose sensitive system information (paths, PIDs of other users' processes, etc.) to the unprivileged caller.

**Mitigation:** Error responses contain only generic status codes and safe descriptions. Internal details are logged to the journal (readable only by root or journal group) but not returned over D-Bus.

## Non-Goals

- Protecting against a caller that already has root (they do not need the helper).
- Rate-limiting by user identity (single-user desktop assumption).
- Protecting against kernel-level attacks on D-Bus credential reporting.
- Network-facing attack surface (helper has no network access via systemd sandboxing).

## Systemd Hardening

The helper's maintenance API is D-Bus activated through `maintainer-helper.service`, which has no
`[Install]` section, so it never runs at boot and never becomes a permanent
daemon. The unit applies:

- `ProtectSystem=strict` with `ReadWritePaths=/var/cache/apt /var/log/journal`,
  the only two locations any operation empties
- `ProtectHome=yes` and `PrivateTmp=yes`
- `NoNewPrivileges=yes`
- `IPAddressDeny=any` and `RestrictAddressFamilies=AF_UNIX`; no operation uses
  the network, and the system bus is a unix socket
- `ProtectKernelModules`, `ProtectKernelTunables`, `ProtectKernelLogs`,
  `ProtectControlGroups`, `ProtectClock`
- `RestrictNamespaces`, `RestrictRealtime`, `RestrictSUIDSGID`, `LockPersonality`
- `MemoryMax=128M`, `CPUQuota=25%`, `TasksMax=32`

`ProtectProc` is deliberately left at its default. Terminating a process
revalidates the target by reading `/proc/<pid>/stat`, so hiding other processes
would break the PID-reuse guard described in T2.

The update CLI does not run inside this systemd unit. Its separate boundary is Polkit's exact executable-path action, strict argument validation, a cleared environment, and immediate `exec` into fixed `/usr/bin/apt-get`.

`CapabilityBoundingSet` and `SystemCallFilter` are not yet applied. Both would
narrow a root service usefully, but a wrong set fails as a silent refusal inside
`apt-get` or `journalctl` rather than a clear error, so they need a live
privileged test before being enabled rather than being asserted here untested.

## D-Bus Access Control

Two distinct mechanisms are involved and they are not interchangeable:

- The **system bus policy** (`/usr/share/dbus-1/system.d/`) decides whether a
  message may be delivered to the helper at all. It permits any user to send to
  the helper interface, and permits only root to own the name.
- **Polkit** decides whether the caller may perform the requested action, and
  prompts for a password when the action requires one.

Reaching the helper is intentionally not the authorization boundary. If the bus
policy denied unprivileged callers, the message would be rejected before the
helper started, Polkit would never be consulted, and the user would see a
refusal with no opportunity to authenticate. Restricting delivery would
therefore not add security; it would only remove the ability to authorize.
