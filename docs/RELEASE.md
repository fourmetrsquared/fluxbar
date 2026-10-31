# Release checklist

Use this checklist for a tagged FluxBar release. The repository targets
macOS 12+; Liquid Glass edge deformation and `NSGlassEffectView` are available
on macOS 26+.

The first public release is `0.1.0`. Update the version in `Cargo.toml`,
`Cargo.lock`, `Formula/fluxbar.rb`, and `CHANGELOG.md` together before tagging.

## Before tagging

- [ ] The default branch is green on macOS and Linux CI, including the example
      plugins.
- [ ] The MSRV job passes with Rust 1.89.
- [ ] `./build.sh check` passes on a clean checkout, including
      `cargo test -p fluxbar-cli --test daemon_integration`.
- [ ] `./build.sh docs` reports no broken relative Markdown links.
- [ ] `cargo audit`, `cargo audit --file examples/plugins/hello/Cargo.lock`, and
      `cargo audit --file examples/plugins/workspaces/Cargo.lock` pass.
- [ ] `cargo deny check` and `cargo machete` pass; the example plugin lockfiles
      are checked with `cargo machete` as well.
- [ ] The unsafe-code invariants in [SAFETY.md](SAFETY.md) still match the
      native renderer.
- [ ] The native bar was manually checked on macOS 12/13, macOS 26, Retina and
      non-Retina displays, with Reduce Motion and Reduce Transparency.
- [ ] The workspaces plugin was checked with yabai and AeroSpace.
- [ ] New user-visible behavior is in `CHANGELOG.md` and the Russian guide.
- [ ] Version numbers in `Cargo.toml`, `Cargo.lock`, `Formula/fluxbar.rb`, and
      the changelog agree.
- [ ] No local paths, credentials, generated caches, or `target/` directories
      are tracked.
- [ ] `LICENSE`, `SECURITY.md`, `CONTRIBUTING.md`, and
      `THIRD_PARTY_NOTICES.md` are present and current.

## Local release build

```bash
./build.sh check
./build.sh docs
./scripts/release.sh
tar -tzf target/dist/assets/fluxbar-0.1.0-macos-universal.tar.gz
```

`scripts/release.sh` (also available as `./build.sh dist`) builds both
`x86_64-apple-darwin` and `aarch64-apple-darwin` with
`MACOSX_DEPLOYMENT_TARGET=12.0`, combines them with `lipo`, verifies
`fluxbar --version`, packages the binary with the license and user docs, and
writes these assets under `target/dist/assets/`:

- `fluxbar-<version>-macos-universal.tar.gz`
- `SHA256SUMS`
- `RELEASE_NOTES.md` (extracted from the matching changelog section)
- `fluxbar.rb` (stable Homebrew formula for the tap)

Verify the archive before publishing:

```bash
cd target/dist/assets
shasum -a 256 -c SHA256SUMS
tar -tzf fluxbar-0.1.0-macos-universal.tar.gz
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
- [ ] On a source install, `fluxbar service install`, `fluxbar service status`,
      and `fluxbar service uninstall --remove-logs` start exactly one daemon
      and remove the plist, socket, PID file, and logs.

## Signing

Developer ID signing is optional but recommended. Export the certificate as a
password-protected `.p12`, base64-encode it, and configure these repository
secrets for the release workflow:

| Secret | Description |
| --- | --- |
| `APPLE_CERTIFICATE_P12` | Base64-encoded Developer ID Application certificate (`.p12`). |
| `APPLE_CERTIFICATE_PASSWORD` | Password for the `.p12` file. May be empty. |
| `APPLE_KEYCHAIN_PASSWORD` | Temporary CI keychain password. |
| `APPLE_SIGNING_IDENTITY` | Full identity, for example `Developer ID Application: Example (TEAMID)`. |

The workflow imports the certificate into a temporary keychain and signs the
universal binary with `codesign --options runtime --timestamp`. It removes the
keychain afterwards. Without these secrets the release is built unsigned and
the workflow prints that fact.

For a local signed build:

```bash
FLUXBAR_CODESIGN_IDENTITY="Developer ID Application: Example (TEAMID)" \
  ./scripts/release.sh
```

The script verifies the signature with `codesign --verify --strict` before
packaging.

Notarization is not automated for the bare command-line binary. Homebrew
installs are not quarantined; if you also distribute a browser-downloaded
`.pkg`, `.dmg`, or app bundle, sign and notarize that container separately.

## Publishing

```bash
git status --short             # must be clean
git tag -a v0.1.0 -m "FluxBar 0.1.0"  # use -s when a signing key is configured
git push origin main
git push origin v0.1.0
```

Pushing a `v*` tag runs `.github/workflows/release.yml`:

1. audits the workspace lockfile;
2. runs `./build.sh check` and the Markdown link check on macOS;
3. builds and optionally signs the universal release artifacts;
4. publishes the GitHub release with the tarball, checksums, notes, and stable
   Homebrew formula;
5. if `HOMEBREW_TAP_TOKEN` is configured, commits the generated formula to the
   tap repository configured by the `HOMEBREW_TAP_REPOSITORY` repository
   variable (default `fourmetrsquared/homebrew-tap`).

The workflow can also be started manually from the Actions tab with an
existing tag. Re-running it uploads the current assets to the existing release
with `--clobber`.

### Homebrew tap

`Formula/fluxbar.rb` in this repository is the development/HEAD formula. The
stable formula is generated at release time from
`packaging/homebrew/fluxbar.rb.template` and points to the universal archive,
so its `sha256` always matches the published asset.

If the tap token is not configured, copy the generated formula manually:

```bash
cp target/dist/assets/fluxbar.rb ../homebrew-tap/Formula/fluxbar.rb
cd ../homebrew-tap
git add Formula/fluxbar.rb
git commit -m "fluxbar 0.1.0"
git push
```

Then verify the managed service from a clean user account:

```bash
brew tap fourmetrsquared/tap
brew trust fourmetrsquared/tap  # only if Homebrew requires tap trust
brew audit --strict --formula fluxbar
brew install fluxbar
brew services start fluxbar
brew services list
brew services restart fluxbar
brew services stop fluxbar
```

- [ ] The service logs are present, the bar is visible, and no duplicate
      FluxBar process is running after `brew services start`.
- [ ] The service starts exactly one bar after a clean login.
- [ ] `brew uninstall fluxbar` removes the binary; `brew services stop` leaves
      the user configuration and caches untouched.

## After publishing

- [ ] Verify the release artifact from a clean machine or VM.
- [ ] Verify the generated `fluxbar.rb` checksum against the release asset.
- [ ] Keep the previous release available for rollback.
- [ ] Triage new dependency advisories in the next scheduled audit.
- [ ] Update `CHANGELOG.md` with a new `[Unreleased]` section before the next
      release.
