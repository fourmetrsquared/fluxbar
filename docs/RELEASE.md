# Release checklist

Use this checklist for a tagged FluxBar release. The repository currently
targets macOS 12+; Liquid Glass edge deformation is available on macOS 26+.

## Before tagging

- [ ] The default branch is green on macOS and Linux CI, including the example
      plugins.
- [ ] `cargo fmt --all -- --check` and
      `cargo clippy --workspace --all-targets -- -D warnings` pass.
- [ ] `./build.sh check` passes on a clean checkout.
- [ ] `cargo audit` and both example lockfile audits pass.
- [ ] The native bar was manually checked on macOS 12/13, macOS 26, Retina
      and non-Retina displays, with Reduce Motion and Reduce Transparency.
- [ ] The workspaces plugin was checked with yabai and AeroSpace.
- [ ] New user-visible behavior is in `CHANGELOG.md` and the Russian guide.
- [ ] Version numbers in `Cargo.toml`, `Cargo.lock`, `Formula/fluxbar.rb`,
      and the changelog agree.
- [ ] No local paths, credentials, generated caches, or `target/` directories
      are tracked.
- [ ] `LICENSE`, `SECURITY.md`, `CONTRIBUTING.md`, and
      `THIRD_PARTY_NOTICES.md` are present and current.

## Build

```bash
./build.sh release
./target/release/fluxbar --version
```

- [ ] The binary starts, shows the bar, and reports the expected version.
- [ ] `fluxbar status` and a restart of the daemon succeed.
- [ ] `fluxbar config reload` applies a valid config, preserves runtime items,
      and rejects an invalid config without changing live state.
- [ ] `SIGINT`, `SIGTERM`, and `fluxbar stop` exit cleanly and remove the
      socket and PID file.
- [ ] The socket and PID file are owner-only:

```bash
stat -f '%Sp %N' ~/Library/Caches/dev.fluxbar.fluxbar/fluxbar.sock
stat -f '%Sp %N' ~/Library/Caches/dev.fluxbar.fluxbar/fluxbar.pid
```

- [ ] A clean profile can run `fluxbar config init` and start without an
      existing configuration.
- [ ] Generate and verify checksums for published artifacts.
- [ ] The Homebrew service starts the bar and survives a clean login:

```bash
brew tap fluxbar/tap
brew trust fluxbar/tap   # only if Homebrew requires tap trust
brew install fluxbar
brew services start fluxbar
brew services list
brew services restart fluxbar
brew services stop fluxbar
```

- [ ] The service logs are present, the bar is visible, and no duplicate
      FluxBar process is running after `brew services start`.

## Publishing

- [ ] Create a signed tag (`git tag -s vX.Y.Z`).
- [ ] Publish a GitHub release with the changelog and checksums.
- [ ] Attach a universal macOS binary if the build environment and signing
      credentials are available.
- [ ] If notarization is used, verify the ticket with `spctl --assess`.
- [ ] Add the stable `url` and `sha256` to `Formula/fluxbar.rb`, copy it into
      the `fluxbar/homebrew-tap` repository, and publish the tap.
- [ ] Announce the release and link to the security policy for reports.

## After publishing

- [ ] Verify the release artifact from a clean machine or VM.
- [ ] Keep the previous release available for rollback.
- [ ] Triage new dependency advisories in the next scheduled audit.
