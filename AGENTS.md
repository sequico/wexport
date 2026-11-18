# wexport — project rules

Project law for this repository. The global rules still apply; where these are stricter, these win.

## Dependencies

- **Always the latest published version.** Before adding or touching a dependency, look up what is
  current (`cargo search <crate>`, or `cargo add <crate>` which picks the newest), then use that
  version, and run `cargo update` in the same change.
- **A version held back needs a reason in the commit.** "It compiles" is not a reason; a breaking
  API, an upstream regression, or an advisory with no fixed release is — and the reason names the
  version that will lift it.
- **No dependency without a job.** A crate is added when the standard library or an existing
  dependency cannot do the work, never to save thirty lines.
- **Security and licences are part of the choice**: `cargo deny check` and `cargo audit` run before
  a push, and a crate with an unmaintained or vulnerable release is replaced or updated, not
  ignored.

## Builds

- `Cargo.lock` is committed, and a release is built from it: `cargo build --release --locked`.
- Warnings are errors (`Cargo.toml` denies rustc, rustdoc and clippy), and the library denies
  `unwrap` and `panic` outside tests.

## The gate

The list is in the README, "Verification before a push". It runs once, whole tree, before a push
or a release — never after every change.
