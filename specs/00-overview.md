# fancontrol-rs — Overview

**Spiritual successor to FanControl (Rem0o), rewritten in Rust.**

Public repo: https://github.com/Leyukaka/fancontrol-rs  
Current product line: **v0.2.x** (early public, NCT668x-focused hardware path).

## Vision

A modern, secure, full-featured desktop fan control application for Windows that:

- Gives the user complete control over every controllable fan and sensor
- Uses a modern, safer kernel backend (**PawnIO**) instead of the vulnerable WinRing0
- Is highly extensible via plugins
- Has a clean, responsive native UI
- Is written entirely in Rust (or as close as possible)

## Core Principles

1. **Security first** — Never ship a custom unsigned/vulnerable ring-0 driver. Prefer PawnIO. Never bundle WinRing0.
2. **Spec-driven** — Major decisions and features are documented in `/specs` (and public `docs/` for users) and kept in sync with reality.
3. **Plugin-first architecture** — Core stays thin; hardware support and advanced features live in providers/plugins when possible.
4. **Full-featured, not minimal** — Target feature parity (and beyond) with the original FanControl over time.
5. **User control** — The user is always in control of curves, profiles, and automation. Product default: **PWM writes on** (use `--read-only` for diagnostics).

## Target Platform

- **Primary**: Windows 10 / 11 (x64)
- Future: possible Linux support later (out of scope for v1)

## License

MIT OR Apache-2.0

## Related public docs

| Doc | Role |
|-----|------|
| [README.md](../README.md) | Entry point, build, download |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | PRs, AI contribution policy |
| [docs/SUPPORTED_HARDWARE.md](../docs/SUPPORTED_HARDWARE.md) | Chip / controller matrix |
| [docs/SECURITY.md](../docs/SECURITY.md) | Scans, SHA256, VT notes |
| [docs/SIGNING_AND_DISTRIBUTION.md](../docs/SIGNING_AND_DISTRIBUTION.md) | Releases, signing roadmap |
| [AGENTS.md](../AGENTS.md) | Coding-agent / maintainer map |
