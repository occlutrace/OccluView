# Contributing to OccluView

First: thank you. OccluView is a community project backed by OccluTrace, and
every careful contribution makes dental technicians' days better.

**Read [`AGENTS.md`](AGENTS.md) first** — the build commands, crate map, and
conventions. This document is the practical companion: how to set up, how to
run checks locally, and what a mergeable PR looks like.

## 1. Set up

Requirements:
- Windows 10 1903+ or Windows 11 (we are Windows-first for v1).
- The Rust toolchain pinned in [`rust-toolchain.toml`](rust-toolchain.toml).
  Installing via `rustup` will pick it up automatically.
- Windows SDK (for the shell extension COM headers and `windows-rs`).
- Git ≥ 2.30 (for `--force-with-lease` and signed DCO commits).

```bash
git clone https://github.com/occlutrace/occluview
cd occluview
rustup show          # installs the pinned toolchain
cargo build --workspace
```

Recommended cargo subcommands (one-time):
```bash
cargo install cargo-deny cargo-nextest cargo-binstall
```

## 2. Before you write code

1. **Find or open an issue.** Every non-trivial change starts as an issue so
   the approach can be discussed before the code exists. Don't open a drive-by
   PR for a big feature without one.
2. **Read the relevant ADRs** in [`docs/adr/`](docs/adr/). If your change
   contradicts a past decision, you need a new ADR first.
3. **Branch** from `main`: `feat/<scope>-<topic>` / `fix/<scope>-<topic>` /
   `docs/<topic>`. One concern per branch.

## 3. Local checks (CI runs the same set; do not push until these pass)

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps          # must build without warnings
cargo deny check licenses advisories     # license + security advisory gate
```

Perf gates (run before touching a hot path):
```bash
cargo bench -p occluview-render          # see docs/TESTING.md for budgets
```

## 4. Commit messages — Conventional Commits

```
<type>(<scope>): <imperative subject up to 72 chars>

<body explaining why, not what>

<footer: BREAKING CHANGE, closes #123, etc.>
```

- **types:** `feat fix docs style refactor perf test build ci chore`
- **scopes:** `core formats render shell app cli docs ci`
- **Breaking:** add `!` after type/scope and a `BREAKING CHANGE:` footer.

Examples:
```
feat(formats): fast-path binary STL with mmap streaming
fix(render): clamp exposure before tonemap to avoid NaN on HDR skies
docs(adr): record choice of wgpu over bgfx (ADR-0002)
```

The squash-merge subject becomes the commit on `main`, so craft it carefully.

## 5. DCO — Developer Certificate of Origin

Every commit must be signed off (`git commit -s`), which attests to the DCO
(<https://developercertificate.org>). The CI verifies the `Signed-off-by` line
matches the commit author.

If you forget, fix with `git commit --amend -s` before pushing. We don't use a
CLA; the DCO is sufficient.

## 6. Pull request checklist

The PR template (`.github/pull_request_template.md`) covers this; summary:

- [ ] Linked issue with `brainstorm` + `plan` notes.
- [ ] Branch is rebased on `main`; one concern per PR; target < 600 diff lines.
- [ ] `cargo fmt` / `clippy -D warnings` / `cargo test` / `cargo doc` all green.
- [ ] New behavior has tests; renderer changes have golden-image baselines (or an
      ADR justifying the baseline change).
- [ ] No new `unwrap`/`expect`/`panic!` outside permitted zones (`AGENTS.md` §3).
- [ ] File/complexity limits respected (≤ 500 LOC/file, ≤ 60 LOC/fn, etc.).
- [ ] Public API documented; CHANGELOG updated for user-visible changes.
- [ ] New dependency → ADR filed + `cargo deny` green.
- [ ] Perf path touched → before/after numbers in the PR.
- [ ] If you are an AI agent: the agent rules in `AGENTS.md` §9 are satisfied.

## 7. Review expectations

- Reviewers check **spec compliance** (does it match the plan task?) and
  **quality** (style, layering, error handling, no slop) separately.
- Be kind, be specific, cite file:line. Suggest, don't dictate.
- A maintainer from `CODEOWNERS` must approve for the touched path.
- Two approvals for changes touching `docs/adr/` or `AGENTS.md` itself.

## 8. Release flow (maintainers)

1. Update `CHANGELOG.md`: move `[Unreleased]` to a version, date it.
2. Tag `vX.Y.Z`; CI builds the signed MSI + symbols.
3. The release is reproducible from the tagged commit + pinned toolchain.
4. Sign the shell DLL and the installer (a shell extension loaded into
   `explorer.exe` **must** be signed — unsigned it can be blocked by SmartScreen).

## 9. Getting help

- Open a `question` issue, or use Discussions.
- For security matters, see [`SECURITY.md`](SECURITY.md) — **do not** open a
  public issue.
