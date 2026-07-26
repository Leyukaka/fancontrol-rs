# Contributing to fancontrol-rs

Thanks for your interest. This project controls **real hardware fans** on Windows via **PawnIO**. Mistakes can leave systems hot, noisy, or stuck at wrong duty cycles — contributions are welcome, but the bar for quality and intent is **strict**.

## License

Contributions are dual-licensed under **MIT OR Apache-2.0**, the same as the rest of the project. By submitting a PR, you agree that your contribution may be distributed under either license.

## Code of conduct (short)

Be respectful. No harassment, spam, or bad-faith interactions. Disagreements about technical choices are fine; personal attacks are not. Maintainers may close issues/PRs that waste reviewer time or endanger users.

## Reporting bugs

Open a **GitHub Issue** with:

1. **What you expected** vs **what happened**
2. OS / build (`cargo run -- --version` or tag), and whether you used admin elevation
3. Board / Super I/O if known (e.g. NCT6687D), or output of `detect` / `backend-status` / `sample`
4. Whether `--read-only` was used (writes are on by default; do **not** paste secrets or full dumps of unrelated personal files)

For hardware misreads/miswrites, attach relevant CLI logs (redact serials if any). Prefer reproducible steps over screenshots alone.

## Proposing features

For **non-trivial** features (new chip paths, UI redesigns, packaging, signing, plugins):

1. Open an **issue first** and describe the problem, not only the solution
2. Wait for maintainer feedback before large implementation PRs
3. One clear scope; avoid “while I was here” mega-changes

Drive-by refactors that touch many crates without a linked issue are likely to be closed.

## Pull requests

### Rules

- **One PR = one concern.** Split unrelated fixes.
- Prefer small, reviewable diffs over giant dumps.
- Keep the tree **buildable**: `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` should pass on Windows.
- Run `cargo fmt --all` before push.
- Add or update tests for **core logic** (curves, profiles, channel map, pure helpers). Hardware paths may be hard to unit-test; document manual validation steps instead of inventing results.
- Do **not** weaken the hardware-write gate (`--read-only` / `--allow-hw-write`) without explicit maintainer discussion.
- Do **not** add WinRing0, LibreHardwareMonitor kernel drivers, or any known-vulnerable ring-0 stack. PawnIO only for privileged I/O.

### PR body template (required)

Put this at the **top** of the PR description:

```markdown
## Patch intent
- **Problem:** …
- **Solution:** …
- **Risk:** (hardware write? config format break? false positives? none)
- **How tested:** (commands / machines / mock only)

## AI disclosure
- **AI tools used:** (none | list: e.g. Claude Code, Grok, Cursor, Copilot)
- **I understand every line and can explain it in review:** yes / no
```

PRs without clear patch intent may be closed without deep review.

## AI-assisted contributions (accepted, strict)

AI tools are allowed. Unreviewed AI dumps are **not**.

### Mandatory

1. **Disclose** AI tools used in the PR body (see template).
2. The **human author** must understand every line and be able to explain design choices in review.
3. State **patch intent** (problem → solution → risk) before the code dump narrative.
4. Do not claim hardware validation you did not run on a real machine.
5. Do not invent APIs, PawnIO IOCTLs, chip register maps, or “green” test output.

### Banned / grounds for rejection

- Unreviewed multi-file AI pastes with no local build/test
- Hallucinated crate APIs or fake module names
- Fake hardware test claims (“tested on NCT6687D, RPM went from X to Y”) without evidence
- Silent removal or bypass of write safety gates
- Shipping secrets, certs, or personal absolute paths unrelated to docs examples

Maintainers may **reject or rewrite** any contribution whose intent, quality, or safety story is weak — AI-assisted or not.

## Quality bar

| Check | Expectation |
|-------|-------------|
| Format | `cargo fmt --all -- --check` |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| Tests | `cargo test --workspace` |
| Specs | Non-trivial product/arch changes should update `specs/` and/or `AGENTS.md` Status when reality changes |
| Hardware | Prefer `sample` / `list-sensors` / `list-controls`; writes are on by default — use `--read-only` unless you have a stated plan |

## Local development (Windows host)

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -- list-sensors
cargo run -- --no-hw ui
```

Hardware needs **PawnIO installed**, an **elevated** process for `pawnio_open`, and usually a Defender exclusion for `target\` during local builds (see README). Agents and contributors must **not** disable antivirus or install system drivers without the machine owner’s approval.

## Docs map

| Doc | Role |
|-----|------|
| [README.md](./README.md) | Build, CLI, UI, Defender |
| [docs/SUPPORTED_HARDWARE.md](./docs/SUPPORTED_HARDWARE.md) | Chip support matrix |
| [docs/SIGNING_AND_DISTRIBUTION.md](./docs/SIGNING_AND_DISTRIBUTION.md) | Signing & release packaging |
| [specs/](./specs) | Spec-Driven Design decisions |
| [AGENTS.md](./AGENTS.md) | Rules for coding agents |

## Questions

Open an issue. For security-sensitive hardware write bugs, still use issues unless a private channel is published later — do not open public PoCs that deliberately brick machines.
