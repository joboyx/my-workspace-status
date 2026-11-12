## Summary

<!-- What changes and why. One short paragraph or a few bullets. -->

## Test plan

<!-- Commands you ran and their counts, for example:
- `cargo test --workspace --locked`: N passed, 0 failed
- `cargo test -p workspace-status --features slow-tests --test tui_tty_e2e pty_<name>`: 1 passed
- Verify skill driver: scenario and capture names (UI changes)
-->

## Definition of done

See **Definition of done** in `AGENTS.md`.

- [ ] Local gate is green: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test --workspace --locked`
- [ ] CI checks of this pull request are green (they run the slow suites: PTY e2e, load test)
- [ ] Tests cover the changed behaviour (unit tests, plus a `pty_*` test for keys or screens)
- [ ] Docs rows in **Update the docs with the code** are updated
- [ ] UI change: seen live through the verify skill driver (`scripts/verify-tui.sh`), or n/a
- [ ] Public repository: no private paths, hostnames, other-project ticket keys, tokens or credentials
