# Contributing to Craft-Agent

Thank you for your interest in improving Craft-Agent.
This document explains the expected workflow, code conventions,
and how to run local checks before opening a PR.

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md).
By participating, you agree to uphold a welcoming and respectful environment.

## How to Contribute

1. Fork the repository and create a feature branch from `main`.
2. Run the following checks locally:
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace`
3. Keep changes focused and minimal.
4. Add docs/tests when changing public APIs.
5. Open a PR with a clear description and linked issue.

## Reporting Issues

- Use GitHub Issues for bugs and feature requests.
- Include reproduction steps, logs, and platform details.
