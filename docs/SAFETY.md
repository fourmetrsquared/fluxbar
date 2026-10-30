# Safety and dependency policy

FluxBar is a desktop application that runs with the privileges of the logged-in
user. This page documents the code-level safety boundary and the automated
dependency checks used before a release.

## Unsafe code

The platform-independent crates (`fluxbar-core`, `fluxbar-sdk`, and
`fluxbar-cli`) deny `unsafe_code` in their manifests. The only unsafe code is
the native macOS interop in `fluxbar-macos`:

- **Main thread only.** `BarView`, `BarWindow`, and their AppKit objects use
  `MainThreadOnly`; they are allocated, mutated, and released on the UI thread.
  The daemon communicates with the UI through channels.
- **Objective-C messages.** Every `msg_send!` uses a selector and argument
  types from the public AppKit contract. Standard `alloc`/`init` pairs are
  converted to `Retained<T>` exactly once, so Objective-C ownership follows
  Rust's lifetime rules.
- **Dynamic Liquid Glass.** `NSGlassEffectView` is looked up at runtime with
  `AnyClass::get` and is only used on macOS 26 and later. The `alloc` result,
  the `initWithFrame:` result, and the class lookup are checked before use;
  older systems stay on the painted fallback.
- **CoreText metrics.** Attribute dictionaries contain documented AppKit keys
  and valid object values. The attributed string stays alive while CoreText
  creates the line, and null output pointers request optional metrics only.
- **Framework loading.** `dlopen` receives a static, NUL-terminated AppKit path
  and only forces registration of already-loaded classes; it does not load
  user-controlled code.

`unsafe_op_in_unsafe_fn` is denied workspace-wide, and all unsafe blocks carry
a `SAFETY` comment describing the invariant at the call site. New unsafe code
must be isolated in `fluxbar-macos`, documented here, and covered by the
platform checks in [RELEASE.md](RELEASE.md).

## Dependencies

The following checks run in CI and should also be run before tagging:

```bash
cargo audit
cargo deny check
cargo machete
cargo machete examples/plugins/hello/Cargo.toml examples/plugins/workspaces/Cargo.toml
```

`deny.toml` allows only permissive licenses, rejects unknown registries and
git sources, and treats wildcard dependency requirements as errors. The
`cargo-machete` ignore list is limited to dependencies that are reachable only
through generated Objective-C bindings, with the reason recorded next to the
entry.

Advisories are reported privately as described in [SECURITY.md](../SECURITY.md).
The MSRV is Rust 1.89 and is checked by a dedicated CI job.
