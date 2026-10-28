# Security Policy

## Supported versions

Security fixes go into the latest release. Please update to it before reporting.

## Reporting a vulnerability

**Please do not report security problems in public issues.**

Report them privately through a [GitHub security advisory](https://github.com/ElRobaMichis/rayo/security/advisories/new).
Include what you found, how to reproduce it and what an attacker could do with it. You can expect a first
answer within a week.

Areas that deserve extra care:

- The optional indexing service (`RayoIndexer`), which runs as `SYSTEM` and talks to the app over a named pipe
- “Continue as administrator”, which runs a single operation elevated
- The “default file explorer” integration, which writes per-user registry keys
- Opening archives and previewing untrusted files
