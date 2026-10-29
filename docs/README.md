# Maintainer documentation

Start with the roadmap for the fast, lightweight, native GTK4 GUI goal. Use the research to check evidence, feasibility and limitations.

| Document | Purpose |
|---|---|
| [Development roadmap](plans/development-roadmap.md) | Current work order, file scope, regression cases and release gates |
| [Safety design decisions](plans/safety-design-decisions.md) | Decisions and proof required for package transactions, filesystem mutation and process identity |
| [Engineering research](research/2026-10-30-engineering-review.md) | Code findings, primary sources, comparison, uncertainties and review corrections |
| [Under-100-MB investigation](research/2026-10-30-memory-under-100mb.md) | Exact memory goal, baseline challenge, optimization experiments and proof of feasibility |
| [Threat model](threat-model.md) | Current controls, open threats and evidence needed; no audit certification implied |

Product behavior and release history remain in [README](../README.md) and [CHANGELOG](../CHANGELOG.md). Reusable [security](../tests/system/security.md), [compatibility](../tests/system/compatibility.md) and [accessibility](../tests/system/accessibility.md) procedures stay beside the system tests. They contain no completed results. Do not move them merely to create another copy under docs.

Keep actionable plans in `plans/` and dated investigations in `research/`. Add `validation/` only when real results exist; record commit/lockfile/package checksums, environment, commands, observations and reviewer. D IDs identify work, E IDs evidence, T IDs threats, A IDs safety decisions, and M IDs memory experiments. Link decisions/experiments to work packages; keep one roadmap. Proposed limits and disabled-capability fallbacks require the decisions stated in that roadmap, not an assumed pass.

The superseded August 2026 plan/specs and duplicate 1.2.0 release draft were removed after checking their source/test replacements. Their removal reasons are recorded in the research note; Git retains the historical versions. Keep this index, the threat model, and the referenced test procedures current.
