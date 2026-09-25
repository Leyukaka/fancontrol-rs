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

**Current project choice:** **Certum Open Source Code Signing, SimplySign cloud variant** (selected 2026-09-25, not purchased/wired yet). Until the first signed release actually ships, releases stay **unsigned + SHA256** and docs must not claim otherwise. Integrity checks and scanning are documented in [SECURITY.md](./SECURITY.md).

### Why Certum Open Source

| Point | Notes |
|-------|-------|
| Cost | Cheapest publicly trusted Authenticode option (tens of USD/year) |
| Trust | OV certificate, Certum root trusted by Microsoft. **Not EV**: SmartScreen reputation starts at zero and builds with downloads |
| Eligibility | Natural person + non-commercial open source project (MIT/Apache on GitHub qualifies). ID + address proof |
| Publisher shown | `Open Source Developer, <maintainer legal name>`: the real name is visible in the exe signature (accepted) |
| Validity | Max 460 days (CA/B Forum, since March 2026): plan a yearly renewal |
| Key storage | SimplySign cloud HSM (no USB token), usable from GitHub-hosted runners |

Rejected: Azure Artifact Signing (individuals limited to US/Canada), SignPath Foundation (signs as "SignPath Foundation", heavier onboarding), smart card variant (no CI signing).

### Planned signing flow

1. Buy **[Open Source Code Signing in the Cloud](https://shop.certum.eu/open-source-code-signing-on-simplysign.html)** (49 EUR as of 2026-09, often out of stock; **not** the 25 EUR card variant). Identity check + project link, then activate SimplySign. **Keep the TOTP QR code/seed shown at activation**: CI login needs it as a secret.
2. Local dry run: `signtool sign /fd sha256 /tr http://time.certum.pl /td sha256 /n "Open Source Developer" fancontrol-rs.exe` then `signtool verify /pa /v fancontrol-rs.exe`.
3. CI: add a **Sign** step in `release.yml` between build and SHA256 staging. SimplySign credentials (login + TOTP seed) live only in the `release` environment secrets. `signtool verify /pa` must fail the job before publishing. If secrets are absent (forks, test dispatch), skip signing with an explicit warning.
4. Compute SHA256 **after** signing.
5. Test via `workflow_dispatch` before the first signed tag, then update README / SECURITY.md to state that releases are signed.

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
