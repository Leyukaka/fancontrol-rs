# Signing and distribution

Practical guide for maintainers. Goal: ship **Windows binaries** users can download without needlessly fighting SmartScreen - without committing secrets or over-engineering CI.

## Why sign?

| Without signing | With Authenticode + good reputation |
|-----------------|-------------------------------------|
| SmartScreen “unknown publisher” | Fewer scary prompts over time |
| Defender heuristics on PawnIO-related binaries | Still possible, but reputation helps |
| Users must trust a random `.exe` | Chain of trust: publisher → timestamp → release tag |

Signing does **not** replace:

- Keeping **PawnIO as a prerequisite** (not embedding foreign kernel stacks)
- **Clear write-vs-read-only messaging** (PWM writes on by default; `--read-only` opts out)
- Publishing **SHA256** checksums next to assets

---

## Options (pick later)

| Option | Pros | Cons / notes |
|--------|------|----------------|
| **Unsigned + SHA256 first** | Zero cost; honest for early OSS | SmartScreen friction; document Defender exclusions for *dev*, not end-user policy |
| **Azure Trusted Signing** | Cloud signing, no local `.pfx` lying around | Account / identity setup; Azure billing/eligibility |
| **OV / EV code signing cert** | Classic Authenticode | Cost, USB token (EV), org validation |
| **[SignPath](https://signpath.io/) for OSS** | Free tier for open source; CI-oriented | Application / policy review |

**Recommendation for early public tags:** ship **unsigned** release artifacts with a **SHA256** file, document PawnIO + admin + SmartScreen honestly, then add Trusted Signing or SignPath when ready for broader audience.

**Current project choice:** remain **unsigned + SHA256** until a maintainer configures a signing provider. Integrity checks and scanning are documented in [SECURITY.md](./SECURITY.md).

---

## Pipeline shape

```text
  tag vX.Y.Z  ──►  GitHub Actions (windows-latest)
                         │
                         ▼
                 cargo build --release -p fancontrol-rs
                         │
                         ▼
              (optional) Authenticode sign + timestamp
                         │
                         ▼
              GitHub Release assets: fancontrol-rs.exe + .sha256
```

Implementation today: **[`.github/workflows/release.yml`](../.github/workflows/release.yml)** - build + upload only. Signing steps are **not** wired yet (no secrets required to land the workflow).

### Who can release (maintainer control)

| Control | Effect |
|---------|--------|
| Branch protection on `main` | No force-push / no delete; PRs need green `Test (Windows)` + `cargo audit` |
| Tag ruleset `v*` | Only **repository admins** can create/update/delete version tags |
| Environment **`release`** | Workflow waits for **owner approval** before build/publish |

So a random collaborator (if ever granted write) cannot silently ship a tagged exe: tags are admin-gated, and publishing waits for your approval in the Actions UI (**Review deployments**).

### GitHub Actions format note

GitHub Actions **requires YAML** under `.github/workflows/`. That is the native platform format - not optional and not a “generator” like cargo-dist. Keep workflows **minimal**: checkout → toolchain → cache → build → artifact/release. Prefer small, readable YAML over large generated matrices until packaging needs grow.

---

## Secrets and certificates

| Do | Do not |
|----|--------|
| Store certs/tokens in **GitHub Actions secrets** or a signing SaaS | Commit `.pfx`, `.p12`, private keys, or password files |
| Use a **timestamp server** when signing (long-term trust after cert expiry) | Rely only on a leaf cert without timestamp |
| Rotate credentials if leaked | Paste secrets into issues, PR bodies, or agent chats |
| Restrict who can approve release workflows | Grant broad org write to every bot |

Never put signing material in the repo, even “temporarily”.

---

## Release checklist (before going public)

- [ ] Version / tag matches `vMAJOR.MINOR.PATCH` (workflow: `v*.*.*`)
- [ ] `cargo test --workspace` and clippy clean on `main`
- [ ] README / SUPPORTED_HARDWARE match real validation status (no fake “signed” claims)
- [ ] PawnIO documented as **prerequisite**; no WinRing0 anywhere
- [ ] Release notes: what’s new, hardware caveats, admin requirement
- [ ] Asset: `fancontrol-rs.exe` (+ `*.sha256` if generated)
- [ ] If signing is enabled: timestamp succeeded; secrets only via GH/SaaS
- [ ] Smoke on a real machine: elevated `backend-status` / `sample` (read-only) before advertising write support
- [ ] **Do not** claim code-signed binaries until signing is actually configured

---

## What end users still need

1. Install **[PawnIO](https://pawnio.eu/)** separately.  
2. Run with **Administrator** rights for Super I/O.  
3. Accept that early releases may be **unsigned** - SmartScreen warnings are expected until reputation/signing exists.  
4. Prefer official **GitHub Releases** over random mirrors; verify SHA256 when published.

---

## Related

- [README.md](../README.md) - Defender / build from source  
- [CONTRIBUTING.md](../CONTRIBUTING.md) - PR and AI policy  
- [docs/SUPPORTED_HARDWARE.md](./SUPPORTED_HARDWARE.md) - chip matrix  
