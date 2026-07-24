# Security

How this project is protected, what we ship, and how to verify downloads.

## Reporting a vulnerability

1. Prefer a **private** report if the issue could be abused (PWM / hardware misuse, RCE, secret leak).  
   On GitHub: **Security → Advisories → Report a vulnerability** (if enabled), or open a minimal public issue **without** exploit details and ask for a private channel.
2. **Do not** paste tokens, private keys, or full dumps of personal configs into issues or PRs.
3. For routine bugs (UI crash, wrong sensor label), a normal [issue](https://github.com/Leyukaka/fancontrol-rs/issues) is fine.

## Branch, tags, and releases (who decides)

| Control | Who |
|---------|-----|
| Force-push / delete `main` | Blocked |
| Merge PR to `main` | Needs green CI (`Test (Windows)`, `cargo audit`) |
| Create tags `v*` | **Repository admin only** (owner) |
| Publish Release exe (workflow) | **Owner approval** on GitHub environment `release` |

You are the sole release decision-maker: even after a tag exists, the Release workflow waits for your **Review deployments** click before uploading `fancontrol-rs.exe`.

## Automated scanning (public repository)

| Mechanism | What it does |
|-----------|----------------|
| **Secret scanning** | GitHub scans public repos for known credential patterns (free on public repos). Enable **push protection** in repo Settings → Code security if available. |
| **CodeQL** | `.github/workflows/codeql.yml` — semantic analysis (Rust + Actions workflows). Alerts under the **Security** tab. |
| **cargo audit** | `.github/workflows/security.yml` — RustSec advisory DB vs `Cargo.lock`. Config: `.cargo/audit.toml`. |
| **Dependabot** | `.github/dependabot.yml` — weekly PRs for Cargo crates and GitHub Actions. |
| **CI** | `fmt` / `clippy -D warnings` / `test` / release build on Windows (`.github/workflows/ci.yml`). |

These reduce risk; they do **not** prove the binary is free of bugs or malware.

### cargo-audit ignores

`.cargo/audit.toml` may ignore specific RustSec IDs when they apply only to **non-Windows** dependency paths (e.g. Linux accessibility via `accesskit_unix` / `zbus_xml` → old `quick-xml`, pulled transitively by `egui-winit` but not used on the Windows ship path). Revisit ignores on every major `eframe`/`egui` upgrade.

## Release integrity (SHA256)

Each tagged Release should include:

- `fancontrol-rs.exe`
- `fancontrol-rs.exe.sha256` (hash of that exe, produced in CI)

### Verify on Windows (PowerShell)

```powershell
Get-FileHash .\fancontrol-rs.exe -Algorithm SHA256
Get-Content .\fancontrol-rs.exe.sha256
```

The hex digests must match (case-insensitive). Prefer downloads only from:

**https://github.com/Leyukaka/fancontrol-rs/releases**

### What SHA256 is — and is not

| SHA256 checksum | Authenticode code signing |
|-----------------|---------------------------|
| Proves the file matches the **published** hash | Proves a **certificate holder** signed the binary |
| Trust rests on **HTTPS + official GitHub release** | Helps Windows SmartScreen / publisher reputation |
| **Already shipped** on releases | **Not configured yet** (see [SIGNING_AND_DISTRIBUTION.md](./SIGNING_AND_DISTRIBUTION.md)) |

A matching SHA256 from the official Release is a strong check against a corrupted or casually swapped file. It is **not** a substitute for code signing if the release channel itself were compromised.

## Code signing (not yet)

Binaries are **not** Authenticode-signed today. Do not claim otherwise.  
When signing is added (SignPath / Azure Trusted Signing / OV-EV cert), it will be documented here and in the release notes.

## Auto-update (roadmap)

**Not implemented yet.** Planned direction:

- Manual **“Check for updates”** in the app (not silent auto-install in v1)
- Query **only** the official GitHub Releases API for this repository
- Download `fancontrol-rs.exe` + `.sha256` and **refuse** install if the hash mismatches
- Optional later: verify Authenticode if/when signing exists

Until then: update by downloading a new Release and verifying SHA256 yourself.

## VirusTotal / Defender behavioral false positives

Unsigned Windows tools that touch **kernel I/O (PawnIO)** and spawn **host helpers** often look like malware under sandbox rules.

### What fancontrol-rs actually does

| Behavior | Why (legitimate) |
|----------|------------------|
| Load PawnIO / talk to Super I/O | Fan/temp hardware access (not WinRing0) |
| Spawn `nvidia-smi` | Optional GPU temperature (read-only) |
| Open `\\.\PhysicalDriveN` + `DeviceIoControl` | Optional SSD/HDD temperature (**no PowerShell**; Win10+ storage stack) |
| Run elevated | Required for `pawnio_open` on many systems |

### Rules you may see (and why they fire)

Examples reported against early builds:

| Behavioral rule (examples) | Likely trigger |
|----------------------------|----------------|
| PowerShell / **SolarMarker**-style (older builds) | Early builds spawned PowerShell for SSD temps — **removed**; storage uses native `DeviceIoControl` now |
| **Change PowerShell Policies…** (older builds) | Was `-ExecutionPolicy Bypass` — **gone** with PowerShell removal for host storage |
| **Unsigned image loaded into LSASS** | Often sandbox / third-party noise; our app does not inject into LSASS. **Code signing** later reduces this class of alerts |
| **PowerShell deleted mounted share** (older builds) | Sandbox FP around storage cmdlets — no longer applicable to storage path |

### What we do / don’t do

- We **do not** download remote scripts, open reverse shells, or disable Defender.
- Prefer official [GitHub Releases](https://github.com/Leyukaka/fancontrol-rs/releases) + **SHA256** verify ([above](#release-integrity-sha256)).
- **Code signing** (when configured) is the main long-term fix for reputation / SmartScreen — see [SIGNING_AND_DISTRIBUTION.md](./SIGNING_AND_DISTRIBUTION.md).

If a vendor flags a release, open an issue with the VT link and the release tag; maintainers can submit false-positive reports once signing exists.

## Product safety (hardware)

- Prefer **PawnIO** only; never ship WinRing0 or known-vulnerable ring-0 drivers.
- Hardware control is **read-only by default**; PWM writes need `--allow-hw-write`.
- Running elevated is required for Super I/O access — treat the binary as privileged software.

See [SUPPORTED_HARDWARE.md](./SUPPORTED_HARDWARE.md) and [CONTRIBUTING.md](../CONTRIBUTING.md).
