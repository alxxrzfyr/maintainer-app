# Compatibility Test Matrix

Status: Template — complete before v1 release.

## Supported environments

| # | Distribution | Version | Desktop | Display | GTK4 version | Kernel | Notes |
|---|---|---|---|---|---|---|---|
| 1 | Ubuntu | 22.04 LTS (Jammy) | GNOME on Wayland | Wayland | 4.6.x | 5.15+ | Compatibility floor |
| 2 | Ubuntu | 22.04 LTS (Jammy) | GNOME on X11 | X11 | 4.6.x | 5.15+ | X11 fallback |
| 3 | Ubuntu | 24.04 LTS (Noble) | GNOME on Wayland | Wayland | 4.14.x | 6.8+ | Current LTS |
| 4 | Ubuntu | 24.04 LTS (Noble) | GNOME on X11 | X11 | 4.14.x | 6.8+ | |
| 5 | Linux Mint | 22 (current) | Cinnamon on X11 | X11 | 4.x | 6.x | Primary derivative |
| 6 | Pop!_OS | 22.04 (current) | COSMIC/GNOME | X11/Wayland | 4.x | 6.x | Additional derivative |

## Test procedure per environment

1. Clean install or clean VM snapshot
2. Install `.deb` package with `sudo apt install ./maintainer_*.deb`
3. Verify runtime dependencies resolve
4. Launch from application menu
5. Complete smoke test sequence (below)
6. Verify clean uninstall with `sudo apt remove maintainer`

## Smoke test sequence

| Step | Action | Expected | Env 1 | Env 2 | Env 3 | Env 4 | Env 5 | Env 6 |
|---|---|---|:---:|:---:|:---:|:---:|:---:|:---:|
| 1 | Launch application | Window appears within 2s | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 2 | Overview loads | Metrics visible, no crash | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 3 | Navigate all pages | Each page renders | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 4 | CPU/memory metrics | Values update every 2s | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 5 | Storage capacity | Mounts listed correctly | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 6 | Process list | Processes visible, sortable | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 7 | Start storage scan | Progress shown, cancellable | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 8 | Cleanup preview | Categories shown | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 9 | Check updates | APT state shown | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 10 | Diagnostics — services | Failed services or "none" | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 11 | Diagnostics — SMART | Health or "unavailable" | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 12 | Generate report | Report preview shown | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 13 | Copy report | Clipboard works | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 14 | Settings | All controls function | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 15 | Theme switch | Light/dark applied | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |
| 16 | Graceful exit | No crash on close | [ ] | [ ] | [ ] | [ ] | [ ] | [ ] |

## Capability availability per environment

| Capability | Env 1 | Env 2 | Env 3 | Env 4 | Env 5 | Env 6 |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| CPU temperature | | | | | | |
| Battery | | | | | | |
| SMART via UDisks2 | | | | | | |
| PackageKit (APT) | | | | | | |
| Flatpak updates | | | | | | |
| Firmware (fwupd) | | | | | | |
| systemd journal | | | | | | |
| systemd services | | | | | | |

Legend: ✓ = available, ✗ = unavailable (expected), P = permission required, N/A = not applicable

## Package installation notes

| Environment | Dependencies resolve | Post-install hooks | Schema compile | Icons appear | Desktop entry | Notes |
|---|:---:|:---:|:---:|:---:|:---:|---|
| Env 1 | [ ] | [ ] | [ ] | [ ] | [ ] | |
| Env 2 | [ ] | [ ] | [ ] | [ ] | [ ] | |
| Env 3 | [ ] | [ ] | [ ] | [ ] | [ ] | |
| Env 4 | [ ] | [ ] | [ ] | [ ] | [ ] | |
| Env 5 | [ ] | [ ] | [ ] | [ ] | [ ] | |
| Env 6 | [ ] | [ ] | [ ] | [ ] | [ ] | |

## Known limitations

| Environment | Limitation | Workaround |
|---|---|---|
| Ubuntu 22.04 | GTK 4.6 — some newer widget APIs unavailable | Compile-time feature gates |
| ___ | ___ | ___ |

## Sign-off

| Reviewer | Date | Environments tested | Result |
|---|---|---|---|
| ___ | ___ | ___ | ___ |
