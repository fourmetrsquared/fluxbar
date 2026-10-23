#!/usr/bin/env bash
#
# FluxBar build helper — works right after `git clone`.
#
# Usage:
#   ./build.sh              Build (debug) and run the test suite
#   ./build.sh release      Optimised release build
#   ./build.sh install      Build release and install to ~/.local/bin
#   ./build.sh run [args]   Build and run the daemon/CLI
#   ./build.sh test         Run all tests
#   ./build.sh check        fmt --check + clippy + tests (CI equivalent)
#   ./build.sh clippy       Run clippy with warnings denied
#   ./build.sh fmt          Format all sources
#   ./build.sh clean        Remove build artifacts
#   ./build.sh examples     Build the example plugin separately
#   ./build.sh doctor       Print environment diagnostics
#   ./build.sh help         Show this help

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

info()    { printf "${BLUE}==>${NC} %s\n" "$*"; }
success() { printf "${GREEN}✓${NC} %s\n" "$*"; }
warn()    { printf "${YELLOW}!${NC} %s\n" "$*"; }
die()     { printf "${RED}error:${NC} %s\n" "$*" >&2; exit 1; }

usage() {
    sed -n '3,20p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
}

require_cargo() {
    if ! command -v cargo >/dev/null 2>&1; then
        die "cargo not found. Install the Rust toolchain from https://rustup.rs and retry."
    fi
}

ensure_toolchain() {
    require_cargo
    local version
    version="$(rustc --version | awk '{print $2}')"
    local major minor
    major="$(echo "$version" | cut -d. -f1)"
    minor="$(echo "$version" | cut -d. -f2)"
    if (( major < 1 )) || (( major == 1 && minor < 85 )); then
        die "Rust $version is too old; FluxBar needs Rust 1.85+ (edition 2024). Run: rustup update stable"
    fi
}

cmd_build() {
    ensure_toolchain
    info "Building FluxBar (debug)…"
    cargo build --workspace
    build_examples
    success "Build complete: target/debug/fluxbar"
    info "Running tests…"
    cargo test --workspace
    success "All tests passed"
}

build_examples() {
    if [[ -f examples/plugins/hello/Cargo.toml ]]; then
        info "Building example plugin…"
        cargo build --manifest-path examples/plugins/hello/Cargo.toml
        success "Example plugin builds"
    fi
}

cmd_release() {
    ensure_toolchain
    info "Building FluxBar (release)…"
    cargo build --workspace --release
    local size
    size="$(du -h target/release/fluxbar 2>/dev/null | cut -f1 || echo '?')"
    success "Release binary: target/release/fluxbar (${size})"
}

cmd_install() {
    cmd_release
    local prefix="${FLUXBAR_PREFIX:-$HOME/.local/bin}"
    mkdir -p "$prefix"
    install -m 0755 target/release/fluxbar "$prefix/fluxbar"
    success "Installed to $prefix/fluxbar"
    case ":$PATH:" in
        *":$prefix:"*) ;;
        *) warn "$prefix is not in PATH. Add this to your shell profile:"
           printf '    export PATH="%s:$PATH"\n' "$prefix" ;;
    esac
    if [[ ! -f "$HOME/.config/fluxbar/fluxbar.toml" ]]; then
        info "No config found. Create one with:"
        printf '    %s/fluxbar config init\n' "$prefix"
    fi
}

cmd_run() {
    ensure_toolchain
    info "Running FluxBar…"
    cargo run --bin fluxbar -- "$@"
}

cmd_test() {
    ensure_toolchain
    info "Running tests…"
    cargo test --workspace
    success "All tests passed"
}

cmd_check() {
    ensure_toolchain
    info "Checking formatting…"
    cargo fmt --all -- --check
    cargo fmt --manifest-path examples/plugins/hello/Cargo.toml -- --check
    success "Formatting OK"
    info "Running clippy…"
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy --manifest-path examples/plugins/hello/Cargo.toml -- -D warnings
    success "Clippy clean"
    build_examples
    cmd_test
}

cmd_clippy() {
    ensure_toolchain
    cargo clippy --workspace --all-targets -- -D warnings
    success "Clippy clean"
}

cmd_fmt() {
    ensure_toolchain
    cargo fmt --all
    if [[ -f examples/plugins/hello/Cargo.toml ]]; then
        cargo fmt --manifest-path examples/plugins/hello/Cargo.toml
    fi
    success "Formatted"
}

cmd_clean() {
    require_cargo
    cargo clean
    success "Cleaned build artifacts"
}

cmd_doctor() {
    printf "%sFluxBar build doctor%s\n" "$BOLD" "$NC"
    printf "  repo:      %s\n" "$SCRIPT_DIR"
    if command -v rustc >/dev/null 2>&1; then
        printf "  rustc:     %s\n" "$(rustc --version)"
        printf "  cargo:     %s\n" "$(cargo --version)"
    else
        printf "  rustc:     ${RED}not found${NC}\n"
    fi
    printf "  platform:  %s\n" "$(uname -s) $(uname -m)"
    if [[ "$(uname -s)" == "Darwin" ]]; then
        printf "  macOS:     %s\n" "$(sw_vers -productVersion 2>/dev/null || echo '?')"
        printf "  UI:        native AppKit\n"
    else
        printf "  UI:        headless (native bar requires macOS)\n"
    fi
    if command -v git >/dev/null 2>&1; then
        printf "  git:       %s\n" "$(git --version)"
    fi
    printf "  config:    %s\n" "${FLUXBAR_CONFIG:-$HOME/.config/fluxbar/fluxbar.toml}"
}

main() {
    local command="${1:-all}"
    shift || true
    case "$command" in
        all|build)   cmd_build ;;
        release)     cmd_release ;;
        install)     cmd_install ;;
        run)         cmd_run "$@" ;;
        test)        cmd_test ;;
        check)       cmd_check ;;
        examples)    build_examples ;;
        clippy)      cmd_clippy ;;
        fmt|format)  cmd_fmt ;;
        clean)       cmd_clean ;;
        doctor)      cmd_doctor ;;
        help|-h|--help) usage ;;
        *)           die "unknown command '$command' (try: ./build.sh help)" ;;
    esac
}

main "$@"
