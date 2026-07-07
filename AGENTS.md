AGENTS.md
=========

General
-------
Follow established patterns and conventions.
Ask Questions, don't assume.

Code Style
----------
- We are using Rust and cargo.
- Use functional patterns where possible.
- No unsafe code if at all possible, if unsafe is necessary the reasons must be clearly documented inline.
- Avoid unnecessary changes, if you identify issues add an entry to `TO-FIX.md`.

Validation
----------
As part of ANY change, all of the following MUST be run and pass before
considering the work complete:
- Formatting: `cargo fmt --all -- --check`
- Lints: `cargo clippy --all-targets --workspace -- -D warnings`
- Tests: `cargo test --workspace`
- Docs: `cargo doc --workspace --no-deps`
- Release build: `cargo build --release`

If any of these fail, fix the issues before finishing. Do not leave the
workspace in a state where any of these checks fail.

Test Logging
------------
The agent's integration tests (`agent/tests/end_to_end.rs`) initialize
logging via the `simple-test-logging` crate
(`simple_test_logging::init()` at the top of each test), pulled in as a
git dev-dependency (https://github.com/tiash/simple-test-logging.git).
The log level is read once from the `LOG_LEVEL` environment variable
(parsed as a `log::LevelFilter`).

- Default (unset or invalid): `error` — keeps test output quiet by default.
- Accepted values (case-insensitive): `off`, `error`, `warn`, `info`,
  `debug`, `trace`.
- Override per-run, e.g. to debug a single test:

      LOG_LEVEL=debug cargo test -p minimal-vm-exec-agent

Note: the level is read once at the first `init()` call in the process and
applied globally for that test binary; changing `LOG_LEVEL` mid-run has no
effect.

Nix packaging note
------------------
`agent/package.nix` builds the agent with `buildRustPackage`. Because the
agent depends on `simple-test-logging` via a **git** dev-dependency,
nixpkgs' `importCargoLock` needs an explicit `outputHash` for it
(`Cargo.lock` only stores the rev, not a content hash). When
`simple-test-logging` is bumped to a new revision:

1. Update the rev in `agent/Cargo.toml` (or let `cargo update` do it) and
   regenerate `Cargo.lock`.
2. Recompute the `outputHash` for `"simple-test-logging-0.1.0"` in
   `agent/package.nix` — set it to `pkgs.lib.fakeSha256`, run
   `nix-build agent/package.nix`, and copy the `got: sha256-...` value.
3. Rebuild to confirm: `nix-build agent/package.nix`.

Git Hooks
---------
The full validation suite above runs automatically via `pre-commit` and
`pre-push` hooks. The hook scripts live in `.githooks/` (version-controlled)
and are shared across clones via `git config core.hooksPath .githooks`.

One-time setup per fresh clone:
    git config core.hooksPath .githooks

Both hooks run the full 5-check suite (pre-commit AND pre-push). A push
immediately after a clean commit will re-run everything; bypass with
`SKIP=all git push` if you just ran pre-commit.

Bypass individual checks (comma-separated, case-insensitive):
    SKIP=fmt,clippy git commit
    SKIP=test,release git push
    SKIP=all git commit          # skip the entire suite

Valid SKIP names: `fmt`, `clippy`, `test`, `doc`, `release`, `all`.
Hooks are bash 3.2-compatible (stock macOS).

Note: the agent tests build and run the actual `minimal-vm-exec-agent`
binary (no VM), so a `cargo test` run is fast (~seconds); the release
build dominates hook runtime (~30-60s).
