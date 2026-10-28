# Third-party notices and attribution

FluxBar is an independent implementation. Its source tree does not vendor
third-party application code. The projects below are referenced through their
public command-line interfaces or APIs for compatibility. Their names and
links are provided so users can find the original projects and their licenses.

| Project | Author / maintainer | Link | How FluxBar uses it |
| --- | --- | --- | --- |
| SketchyBar | Felix Kratz | <https://github.com/FelixKratz/SketchyBar> | Command and event compatibility. `fluxbar-cli/src/sketchybar.rs` is an independent translation layer; no SketchyBar source code is copied. |
| yabai | Koekeishiya | <https://github.com/koekeishiya/yabai> | The optional workspaces plugin calls the `yabai` CLI and reads its JSON output. No yabai source code is copied. |
| AeroSpace | Nikita Bobko | <https://github.com/nikitabobko/AeroSpace> | The optional workspaces plugin calls the AeroSpace CLI and reads its tab-separated output. No AeroSpace source code is copied. |
| AppKit / Liquid Glass | Apple Inc. | <https://developer.apple.com/documentation/appkit> | The native renderer uses public macOS frameworks, including `NSGlassEffectView` on macOS 26 and later. These are system frameworks, not vendored code. |

## Rust dependencies

The Rust crates used by FluxBar are listed in `Cargo.lock`. They remain under
their respective upstream licenses. A dependency audit can be run with:

```bash
cargo audit
```

If a future change copies or adapts a third-party source file, add an
attribution comment with the author, repository URL, license, and a description
of the adaptation, then update this file. See [CONTRIBUTING.md](CONTRIBUTING.md).
