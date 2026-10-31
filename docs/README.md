# Documentation

| Document | Owns |
|---|---|
| [README](../README.md) | Product overview, setup, and commands |
| [Contributor instructions](../AGENTS.md) | Development rules and performance acceptance |
| [Release status](release-status.md) | Current scope, remaining work, and verification links |
| [Threat model](threat-model.md) | Trust boundaries, controls, and residual risks |
| [Validation records](validation/README.md) | Dated observations for specific artifacts |
| [Local logs](../logs/README.md) | Private raw output, ignored by Git |
| [Archive](archive/README.md) | Read-only historical plans and research |

[CHANGELOG](../CHANGELOG.md) records release history. Manual procedures stay in
[security](../tests/system/security.md), [compatibility](../tests/system/compatibility.md),
and [accessibility](../tests/system/accessibility.md) system tests.

Verify behavior against code. Update the document that owns a fact and link to it
elsewhere; do not copy commands, mutable status, or policy into multiple guides.
Do not rewrite dated observations to make a newer revision appear verified.
