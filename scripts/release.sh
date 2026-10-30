#!/usr/bin/env bash
#
# Build FluxBar release artifacts on macOS.
#
# Usage:
#   scripts/release.sh [--output DIR] [--no-sign]
#
# The script builds a universal (Apple Silicon + Intel) binary, packages it
# with the license and user documentation, writes SHA256SUMS, extracts the
# matching CHANGELOG section, and renders the stable Homebrew formula used by
# the release tap.
#
# Environment:
#   FLUXBAR_CODESIGN_IDENTITY  Developer ID Application identity. When set,
#                              the universal binary is signed with a hardened
#                              runtime and a secure timestamp.
#   FLUXBAR_DIST_DIR           Output directory (default: target/dist).
#   FLUXBAR_REPO               GitHub owner/repo for release URLs
#                              (default: fluxbar/fluxbar, or GITHUB_REPOSITORY).
#   MACOSX_DEPLOYMENT_TARGET   Minimum macOS version (default: 12.0).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()    { printf "${BLUE}==>${NC} %s\n" "$*"; }
success() { printf "${GREEN}✓${NC} %s\n" "$*"; }
warn()    { printf "${YELLOW}!${NC} %s\n" "$*"; }
die()     { printf "${RED}error:${NC} %s\n" "$*" >&2; exit 1; }

usage() {
    awk 'NR > 2 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"
    exit 0
}

OUTPUT="${FLUXBAR_DIST_DIR:-$ROOT/target/dist}"
SIGN=1

while [[ $# -gt 0 ]]; do
    case "$1" in
        --output)
            [[ $# -ge 2 ]] || die "--output requires a directory"
            OUTPUT="$2"
            shift 2
            ;;
        --output=*)
            OUTPUT="${1#*=}"
            shift
            ;;
        --no-sign)
            SIGN=0
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            die "unknown argument '$1' (try: scripts/release.sh --help)"
            ;;
    esac
done

[[ "$(uname -s)" == "Darwin" ]] || die "release artifacts must be built on macOS"
for tool in cargo rustup lipo shasum tar gzip; do
    command -v "$tool" >/dev/null 2>&1 || die "required tool not found: $tool"
done

VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
[[ -n "$VERSION" ]] || die "could not read the workspace version from Cargo.toml"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] || die "invalid release version: $VERSION"

FORMULA_VERSION="$(sed -n 's/^  version "\(.*\)"/\1/p' "$ROOT/Formula/fluxbar.rb" | head -n 1)"
[[ "$FORMULA_VERSION" == "$VERSION" ]] \
    || die "Formula/fluxbar.rb is $FORMULA_VERSION, but Cargo.toml is $VERSION"
LOCK_VERSION="$(awk '
    /^name = "fluxbar-cli"$/ { found = 1; next }
    found && /^version = / { gsub(/"/, "", $3); print $3; exit }
' "$ROOT/Cargo.lock")"
[[ "$LOCK_VERSION" == "$VERSION" ]] \
    || die "Cargo.lock has fluxbar-cli $LOCK_VERSION, but Cargo.toml is $VERSION"
grep -q "^## \[$VERSION\]" "$ROOT/CHANGELOG.md" \
    || die "CHANGELOG.md has no section for $VERSION"

REPO="${FLUXBAR_REPO:-${GITHUB_REPOSITORY:-fluxbar/fluxbar}}"
DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-12.0}"
TARGETS=(x86_64-apple-darwin aarch64-apple-darwin)
NAME="fluxbar-${VERSION}-macos-universal"
URL="https://github.com/${REPO}/releases/download/v${VERSION}/${NAME}.tar.gz"

TARGET_ROOT="${CARGO_TARGET_DIR:-$ROOT/target}"
[[ "$TARGET_ROOT" = /* ]] || TARGET_ROOT="$ROOT/$TARGET_ROOT"

mkdir -p "$OUTPUT"
OUTPUT="$(cd "$OUTPUT" && pwd)"
ASSETS="$OUTPUT/assets"
STAGE_ROOT="$OUTPUT/stage"
STAGE="$STAGE_ROOT/$NAME"

rm -rf "$ASSETS" "$STAGE_ROOT"
mkdir -p "$ASSETS" "$STAGE"

info "Preparing universal FluxBar ${VERSION} for macOS ${DEPLOYMENT_TARGET}+"

for target in "${TARGETS[@]}"; do
    if ! rustup target list --installed | grep -qx "$target"; then
        info "Installing Rust target $target"
        rustup target add "$target"
    fi
done

for target in "${TARGETS[@]}"; do
    info "Building ${target}…"
    MACOSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" \
        cargo build --locked --release --target "$target" -p fluxbar-cli
done

info "Creating universal binary…"
lipo -create \
    "$TARGET_ROOT/x86_64-apple-darwin/release/fluxbar" \
    "$TARGET_ROOT/aarch64-apple-darwin/release/fluxbar" \
    -output "$STAGE/fluxbar"
chmod 0755 "$STAGE/fluxbar"

lipo -info "$STAGE/fluxbar"
file "$STAGE/fluxbar"

REPORTED_VERSION="$("$STAGE/fluxbar" --version)"
[[ "$REPORTED_VERSION" == "fluxbar $VERSION" ]] \
    || die "binary reports '$REPORTED_VERSION', expected 'fluxbar $VERSION'"
success "Universal binary reports $REPORTED_VERSION"

if [[ "$SIGN" == "1" && -n "${FLUXBAR_CODESIGN_IDENTITY:-}" ]]; then
    info "Signing with '${FLUXBAR_CODESIGN_IDENTITY}'"
    codesign --force --options runtime --timestamp \
        --sign "$FLUXBAR_CODESIGN_IDENTITY" "$STAGE/fluxbar"
    codesign --verify --strict --verbose=2 "$STAGE/fluxbar"
    success "Code signature verified"
elif [[ "$SIGN" == "0" ]]; then
    warn "Code signing disabled with --no-sign"
else
    warn "Building an unsigned binary (set FLUXBAR_CODESIGN_IDENTITY to sign)"
fi

for file in LICENSE README.md CHANGELOG.md SECURITY.md THIRD_PARTY_NOTICES.md; do
    [[ -f "$ROOT/$file" ]] || die "missing release file: $file"
    cp "$ROOT/$file" "$STAGE/$file"
done

info "Creating $NAME.tar.gz…"
ARCHIVE="$ASSETS/$NAME.tar.gz"
TAR_FILE="$ASSETS/$NAME.tar"
rm -f "$ARCHIVE" "$TAR_FILE"
COPYFILE_DISABLE=1 tar --no-xattrs -cf "$TAR_FILE" -C "$STAGE" .
gzip -n -9 -f "$TAR_FILE"

ARCHIVE_SHA="$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"

info "Writing release notes…"
NOTES="$ASSETS/RELEASE_NOTES.md"
CHANGELOG_SECTION="$(awk -v ver="$VERSION" '
    index($0, "## [" ver "]") == 1 { found = 1; next }
    found && substr($0, 1, 4) == "## [" { exit }
    found { print }
' "$ROOT/CHANGELOG.md")"
[[ -n "${CHANGELOG_SECTION//[[:space:]]/}" ]] \
    || die "could not extract the CHANGELOG section for v$VERSION"
{
    printf '# FluxBar %s\n\n' "$VERSION"
    printf 'Universal macOS binary for Apple Silicon and Intel (macOS %s or later).\n\n' "$DEPLOYMENT_TARGET"
    printf '## Install\n\n```bash\n'
    printf 'brew tap fluxbar/tap\n'
    printf 'brew install fluxbar\n'
    printf 'brew services start fluxbar\n'
    printf '```\n\n'
    printf 'Or download `%s.tar.gz` from this release, verify `SHA256SUMS`,\n' "$NAME"
    printf 'and install the `fluxbar` binary somewhere in your `PATH`.\n\n'
    printf '## Changes\n\n%s\n\n' "$CHANGELOG_SECTION"
    printf '## Verification\n\n```bash\n'
    printf 'shasum -a 256 -c SHA256SUMS\n'
    printf '```\n'
} > "$NOTES"
grep -q '[^[:space:]]' "$NOTES" || die "release notes are empty"

info "Rendering stable Homebrew formula…"
FORMULA="$ASSETS/fluxbar.rb"
TEMPLATE="$ROOT/packaging/homebrew/fluxbar.rb.template"
[[ -f "$TEMPLATE" ]] || die "missing Homebrew template: $TEMPLATE"
sed \
    -e "s|@@VERSION@@|$VERSION|g" \
    -e "s|@@SHA256@@|$ARCHIVE_SHA|g" \
    -e "s|@@URL@@|$URL|g" \
    "$TEMPLATE" > "$FORMULA"
if grep -q '@@' "$FORMULA"; then
    die "unsubstituted placeholder remains in the generated Homebrew formula"
fi
if command -v ruby >/dev/null 2>&1; then
    ruby -c "$FORMULA" >/dev/null
    success "Homebrew formula syntax is valid"
fi

info "Writing and verifying SHA256SUMS…"
(
    cd "$ASSETS"
    shasum -a 256 "$NAME.tar.gz" fluxbar.rb RELEASE_NOTES.md > SHA256SUMS
    shasum -a 256 -c SHA256SUMS
)

rm -rf "$STAGE_ROOT"

success "Release artifacts for FluxBar ${VERSION}:"
find "$ASSETS" -maxdepth 1 -type f -print | sort | while IFS= read -r file; do
    printf '    %s (%s)\n' "${file#"$ROOT/"}" "$(du -h "$file" | cut -f1)"
done
printf '\nStable Homebrew formula: %s\n' "${FORMULA#"$ROOT/"}"
printf 'Release URL: %s\n' "$URL"
