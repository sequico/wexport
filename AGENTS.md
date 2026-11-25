# wexport — project rules

Project law for this repository; where it is stricter than the general rules, it wins.

## No history in the artifacts

Code, `-h`, command output, README, docs and comments describe **what is**, never what was: no "no
longer supported", no "previously", no "the old flag", no "was renamed", no "the first
implementation", no change log in prose. The history is `git log` and `CHANGELOG.md`, and nothing
else.

A reader who has never seen an earlier version must not learn from the artifact that one existed.

## Dependencies

- **Always the latest published version.** Before adding or touching a dependency, look up what is
  current (`cargo search <crate>`, or `cargo add <crate>` which picks the newest), then use that
  version, and run `cargo update` in the same change.
- **A version held back needs a reason in the commit.** "It compiles" is not a reason; a breaking
  API, an upstream regression, or an advisory with no fixed release is — and the reason names the
  version that will lift it.
- **No dependency without a job.** A crate is added when the standard library or an existing
  dependency cannot do the work, never to save thirty lines.
- **Security and licences are part of the choice**: the dependency gate (`cargo deny check`, reading
  `deny.toml`) runs before a push, and a crate with an unmaintained or vulnerable release is replaced
  or updated, not ignored.

## Builds

- `Cargo.lock` is committed, and a release is built from it: `cargo build --release --locked`.
- Warnings are errors (`Cargo.toml` denies rustc, rustdoc and clippy), and the library denies
  `unwrap` and `panic` outside tests.

## The gate

The gate is `scripts/gate.sh` (a push, about twenty seconds) and `scripts/gate-release.sh` (a
release, adding mutation testing, fuzzing, ThreadSanitizer and the `--locked` build). They run once,
whole tree, before a push or a release — never after every change.
