# fancontrol-rs — Overview

**Spiritual successor to FanControl (Rem0o), rewritten in Rust.**

## Vision

A modern, secure, full-featured desktop fan control application for Windows that:

- Gives the user complete control over every controllable fan and sensor
- Uses a modern, safer kernel backend (PawnIO) instead of the vulnerable WinRing0
- Is highly extensible via plugins
- Has a clean, responsive native UI
- Is written entirely in Rust (or as close as possible)

## Core Principles

1. **Security first** — Never ship a custom unsigned/vulnerable ring-0 driver. Prefer PawnIO.
2. **Spec-driven** — All major decisions and features are documented in `/specs` before implementation.
3. **Plugin-first architecture** — Core stays thin; hardware support and advanced features live in plugins when possible.
4. **Full-featured, not minimal** — Target feature parity (and beyond) with the original FanControl.
5. **User control** — The user is always in control of curves, profiles, and automation.

## Target Platform

- **Primary**: Windows 10 / 11 (x64)
- Future: possible Linux support later (out of scope for v1)

## License

MIT OR Apache-2.0
