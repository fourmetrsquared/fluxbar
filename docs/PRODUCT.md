# PRODUCT.md

## FluxBar: Next-Generation Status Bar in Rust
**Slogan**: *Blazing fast. Secure. Absolutely customizable.*

### 1. Vision and Problem
Existing solutions (e.g., SketchyBar) are powerful but have architectural limitations: they rely heavily on executing shell scripts for every single event. This creates significant `fork/exec` overhead, limits data typing, complicates debugging, and makes the system vulnerable to errors in third-party scripts.

**FluxBar** solves this problem by providing a Rust-based core with an event-driven architecture and a modern plugin system. We support both lightweight persistent processes (via IPC) and isolated WebAssembly modules, while maintaining the full flexibility of the "Configuration as Code" concept.

### 2. Target Audience
- **Developers and Power Users**: macOS (and eventually Linux/Wayland) users leveraging tiling window managers (yabai, Aerospace, Hyprland) who value complete control over their workspace.
- **Plugin Authors**: Developers who want to build reliable extensions in familiar languages (Rust, TypeScript, Python, Lua) without the pain of parsing shell arguments or dealing with shell hooks.
- **Customization Enthusiasts**: Users who care about smooth 60+ FPS animations, native OS integration, and zero resource consumption at idle.

### 3. Key Features
- **Full Dynamic Customization**: Add, remove, and modify any bar elements (icons, text, graphs, system menu aliases) in real-time without restarting the bar.
- **Event-Driven Bus**: Built-in system events (`space_change`, `media_change`, `mouse.entered`) and support for custom events (`fluxbar --trigger build_finished`).
- **Smart Plugin System**:
  - Support for persistent connections (Unix Domain Sockets / stdin/stdout) to eliminate `fork/exec` latency.
  - Official SDKs for Rust, Python, and TypeScript.
  - Sandbox for community plugins via WebAssembly (Wasm).
  - Built-in plugin manager (`fluxbar plugin install <repo>`).
- **Advanced Rendering**: Support for Nerd Fonts, SF Symbols, smooth vector graphs, gradients, background blur, and animation interpolation.
- **Full Interactivity**: Native mouse support (click, scroll, enter, exit) bindable to shell commands, actions, or IPC messages.

### 4. High-Level Architecture
1. **Core**: An asynchronous event loop powered by `tokio`, managing the bar's state, animation logic, and rendering.
2. **IPC Layer**: JSON-RPC over Unix Domain Sockets for typed, bidirectional communication between plugins and the CLI utility.
3. **Renderer**: An abstracted rendering layer. For macOS: native integration via `objc2` + Core Animation (for perfect NSStatusItem behavior). For cross-platform readiness: `wgpu` + `cosmic-text`.
4. **Plugin Host**: A lifecycle manager that spawns, monitors, isolates failures, and automatically restarts crashed plugins.

### 5. Value Proposition (Why FluxBar?)
| SketchyBar Limitation | FluxBar Solution |
| :--- | :--- |
| `fork/exec` overhead on every tick | Persistent plugins with constant IPC connection (0 ms spawn overhead) |
| Untyped string arguments in shell scripts | Strict JSON-RPC contracts with `serde` validation |
| Script crashes can break bar logic | Fault isolation: Plugin Host restarts crashed plugins without affecting the core |
| Difficulty in distributing plugins | Built-in CLI package manager and Wasm sandbox support |
