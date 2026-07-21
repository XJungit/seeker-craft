# Release Process

This document describes how to prepare a release of Craft-Agent.

## Versioning

We use semver-like tags for milestones:
- `v0.x.y` for early development
- Breaking changes documented in `CHANGELOG.md`

## Pre-release Checklist

- [ ] `CHANGELOG.md` updated with all changes since last tag
- [ ] `cargo check --workspace` passes (no warnings preferred)
- [ ] `cargo test --workspace` passes
- [ ] `cargo clippy --workspace` clean (or known exceptions documented)
- [ ] `cargo fmt --check` passes
- [ ] Example runs: `agent_multi_step_mod` compiles and starts
- [ ] All crate READMEs reflect current API
- [ ] `docs/tutorials/` consistent with release

## Release Steps

1. Commit all changes, push to `main`.
2. Tag the release:
   ```bash
   git tag -a v0.x.y -m "v0.x.y: <summary>"
   git push origin v0.x.y
   ```
3. Crate publishing (if applicable):
   ```bash
   cargo publish -p craft-agent-model
   cargo publish -p craft-agent
   cargo publish -p craft-agent-minecraft
   cargo publish -p craft-agent-viewer
   ```
4. Verify crates.io listing and README rendering.
