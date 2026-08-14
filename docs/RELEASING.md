# Release Process

This document describes how to prepare a release of SeekerCraft (Craft-Agent).

## Versioning

We use semver-like tags:
- `v1.0.0` — first stable 1.0 release (2026-08-15; DSH bridge mode is the only supported usage)
- Later `v1.x.y` / `v2.x.y` milestones
- Breaking changes documented in [`../CHANGELOG.md`](../CHANGELOG.md) (Keep a Changelog format)

## Pre-release Checklist

- [ ] `../CHANGELOG.md` updated with all changes since last tag
- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets --features craft-agent-minecraft/azalea-bot -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] `cargo test -p craft-agent-minecraft --features azalea-bot --lib` passes
- [ ] `scripts/ci/validate_data_json.py` passes (data JSON validity)
- [ ] No machine-specific paths in tracked files:
  `git grep -nE "D:/Craft-Agent|C:/Users/|anomalyco" -- ':!vendor/azalea' ':!*.lock'`
- [ ] `Cargo.lock` azalea source is `git+https://github.com/XJungit/azalea?...` (NOT `file:///...`)
- [ ] `git submodule status` clean; `vendor/azalea` HEAD matches the manifest rev
- [ ] All crate READMEs reflect current API
- [ ] `docs/tutorials/` consistent with release

## Release Steps

1. Commit all changes, push to `main`:
   ```bash
   git add -A && git commit -m "release: v1.0.0 ..."
   git push origin main
   ```
2. Tag the release:
   ```bash
   git tag -a v1.0.0 -m "v1.0.0: first stable release (DSH bridge mode)"
   git push origin v1.0.0
   ```
3. Verify CI on GitHub Actions (fmt+clippy / test / coverage / audit / docs).
4. Crate publishing (if applicable):
   ```bash
   cargo publish -p craft-agent-model
   cargo publish -p craft-agent
   cargo publish -p craft-agent-minecraft
   cargo publish -p craft-agent-viewer
   ```
5. Verify crates.io listing and README rendering.

## Updating the azalea fork (before a release that depends on new azalea code)

The project depends on the maintained fork `XJungit/azalea` (`craft-agent` branch).
Full workflow: see [`ARCHITECTURE.md`](../ARCHITECTURE.md) → "azalea fork maintenance".
Summary:

1. `git -C vendor/azalea fetch https://github.com/azalea-rs/azalea main`
2. Merge/rebase upstream onto `craft-agent` (keeping the custom archery/equipping APIs)
3. `git -C vendor/azalea push xj HEAD:craft-agent`
4. Update the 6 azalea `rev`s in `crates/craft-agent-minecraft/Cargo.toml`
5. Regenerate `Cargo.lock` from the https source (temporarily move `.cargo/config.toml`
   away, run `cargo update -p azalea`, verify the lock source, restore the patch)
6. Build + test locally; commit the manifest, lock, and `vendor/azalea` gitlink together.
