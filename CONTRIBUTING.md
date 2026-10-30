# Contributing to FluxBar

Thanks for improving FluxBar. This project targets macOS for the native UI and
keeps the core, CLI, and plugin SDK buildable on other Unix-like systems.

## Development setup

Requirements:

- Rust 1.89 or newer (`rust-toolchain.toml` pins a stable toolchain)
- macOS 12+ for the native bar
- yabai or AeroSpace only if you are working on the workspaces example

```bash
git clone https://github.com/fluxbar/fluxbar.git
cd fluxbar
./build.sh check
```

`./build.sh check` runs formatting, clippy with warnings denied, the example
plugin builds, and all workspace tests. To iterate on the native app:

```bash
./build.sh run start
```

## Code style

- Keep the existing module boundaries: platform-independent logic belongs in
  `fluxbar-core`, AppKit interop in `fluxbar-macos`, and command translation in
  `fluxbar-cli`.
- Prefer small, focused changes and add a regression test for every bug fix.
- Avoid unbounded reads, child processes without timeouts, and shell
  interpolation. Pass untrusted values as arguments, not through a shell.
- All public functions and non-obvious unsafe blocks need a short comment
  explaining their contract.
- Run `cargo fmt --all` before opening a pull request.

## Commit and pull request guidelines

- Use an imperative, descriptive commit subject.
- Describe the user-visible behavior and the security impact of the change.
- Link the issue or design discussion when one exists.
- Keep generated files and local build artifacts out of the commit.
- By submitting a pull request you agree that your contribution is licensed
  under the repository's MIT license.

## Third-party code and attribution

Do not copy source code from another project unless its license is compatible
with MIT and you preserve the required notice.

If you copy or adapt code, add a comment at the top of the file or function with:

- the author or maintainer,
- the repository URL,
- the source license,
- a note describing what was adapted.

Then update [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Interoperability
with an external tool (for example yabai or AeroSpace) does not require a code
license, but it should be documented in the relevant module.

## Security-sensitive changes

Changes to IPC, plugin installation, command execution, file permissions, or
the native renderer need extra review. Please include:

- the trust boundary being changed,
- what happens with malformed or hostile input,
- tests for limits and failure paths,
- the output of `cargo audit`, `cargo deny check`, and `cargo machete` when
  dependencies changed.

Unsafe code belongs only in `fluxbar-macos`; document the invariant in a
`SAFETY` comment and update [docs/SAFETY.md](docs/SAFETY.md).

Read [SECURITY.md](SECURITY.md) for the threat model and reporting process.

## Documentation

Keep [README.md](README.md), [docs/GUIDE_RU.md](docs/GUIDE_RU.md), and the
field reference in [docs/CONFIGURATION.md](docs/CONFIGURATION.md) in sync with
user-visible behavior. The Russian guide is the primary onboarding document for
users; English reference pages remain available for plugin authors.
