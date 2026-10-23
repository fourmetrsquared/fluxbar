# REQUIREMENTS.md

## FluxBar Project Requirements (PRD)

### 1. Functional Requirements

#### 1.1. Core & State Management
- [ ] **FR-1.1**: The system must allow adding, removing, and modifying bar items on the fly via CLI or IPC without restarting the main process.
- [ ] **FR-1.2**: Support for flexible item positioning: `left`, `center`, `right`, as well as absolute positioning and stacking.
- [ ] **FR-1.3**: Built-in support for "aliases" (capturing and displaying native macOS menu bar icons, e.g., Wi-Fi, Bluetooth, MeetingBar).

#### 1.2. Event System
- [ ] **FR-2.1**: Generation and handling of built-in events: space changes, app focus, volume, brightness, power state, and media playback.
- [ ] **FR-2.2**: Ability to register and trigger custom events: `fluxbar --trigger my_event --payload '{"status": "ok"}'`.
- [ ] **FR-2.3**: Support for timer-based triggers (polling) with configurable intervals (e.g., updating data every 2 seconds).

#### 1.3. Plugin System
- [ ] **FR-3.1**: Support for **Persistent Plugins**: A plugin spawns once and communicates with the core via JSON-RPC (stdin/stdout or UDS), eliminating process creation overhead.
- [ ] **FR-3.2**: Support for **Wasm Plugins** (v2 goal): Execution of community plugins in a secure sandbox (Wasmer/Wasmtime) with restricted filesystem access.
- [ ] **FR-3.3**: Official `fluxbar-sdk` (Rust crate) that simplifies plugin creation to implementing just 1–2 traits (e.g., `on_event` and `on_tick`).
- [ ] **FR-3.4**: Built-in CLI plugin manager: `fluxbar plugin install github:user/repo`, which automatically clones, builds (if required), and registers the plugin in the config.
- [ ] **FR-3.5**: Fault Tolerance: A plugin crash must not panic the main bar. The host must automatically restart the crashed plugin with exponential backoff.

#### 1.4. Customization & Rendering
- [ ] **FR-4.1**: Support for complex layouts within a single item: icon + text + graph + progress indicator.
- [ ] **FR-4.2**: Smooth animations (interpolation) for width, color, position, and opacity changes. Duration and easing functions (e.g., `ease_out_expo`) must be configurable.
- [ ] **FR-4.3**: Rendering of vector graphs (line graphs, bar charts) based on provided arrays of numerical data.
- [ ] **FR-4.4**: Mouse event handling: `mouse.clicked`, `mouse.scrolled`, `mouse.entered`, bindable to shell commands or IPC messages.

#### 1.5. Configuration
- [ ] **FR-5.1**: Primary configuration in `TOML` format (human-readable, with comment support and strict typing).
- [ ] **FR-5.2**: Modular configuration support: ability to import files from a `~/.config/fluxbar/plugins.d/` directory.

---

### 2. Non-Functional Requirements

- [ ] **NFR-1 (Performance)**: Idle CPU consumption < 0.5%. Event response time < 16 ms (to support 60 FPS animations).
- [ ] **NFR-2 (Reliability)**: The core must handle plugin panics, connection drops, and malformed JSON messages without crashing (graceful degradation).
- [ ] **NFR-3 (Security)**: Plugins must not have direct access to core memory. Wasm plugins must be restricted from filesystem and network access by default (principle of least privilege).
- [ ] **NFR-4 (Cross-Platform Readiness)**: The architecture must abstract rendering and system events (via trait objects) to enable future porting to Linux (Wayland), even if v1 targets macOS exclusively.

---

### 3. Technology Stack

| Component | Technology | Rationale |
| :--- | :--- | :--- |
| **Language** | Rust | Memory safety, high performance, excellent ecosystem for CLI and system software. |
| **Async Runtime** | `tokio` | Mature, high-performance async environment for IPC, timers, and networking. |
| **Serialization** | `serde`, `serde_json` | De facto standard for typed JSON-RPC, minimizing parsing errors. |
| **IPC** | `tokio::net::UnixStream` / `jsonrpsee` | Fast inter-process communication without TCP network overhead. |
| **CLI** | `clap` (with derive) | Powerful, ergonomic, and typed command-line argument parser. |
| **Rendering (macOS)** | `objc2` + Core Animation | Native approach for perfect macOS integration, transparency, and NSStatusItem behavior. |
| **Text & Fonts** | `cosmic-text` | Modern, fast text rendering with support for ligatures, emojis, and complex scripts. |
| **Wasm Host** | `wasmtime` (optional, v2) | Secure and isolated execution of community code. |

---

### 4. Development Milestones

#### 🚩 Milestone 1: MVP (Core & Basic Rendering)
- Initialize the window/status bar on macOS.
- Basic TOML configuration parsing.
- Rendering of static text and icons (SF Symbols / Nerd Fonts).
- Simple CLI for `--add`, `--set`, and `--remove` commands.

#### 🚩 Milestone 2: Events & IPC
- Implement the event bus (subscribing to macOS system events via `notify` or native APIs).
- Implement the JSON-RPC server over Unix Domain Sockets.
- Support for basic change animations (width and color interpolation).

#### 🚩 Milestone 3: Plugin System v1
- Create and publish the `fluxbar-sdk` (Rust crate).
- Implement the host for persistent plugins (auto-restart, logging, disconnect handling).
- Reference plugin implementations: CPU/RAM monitor, Media player (in Rust and Python).

#### 🚩 Milestone 4: Polish & Ecosystem
- Support for graph rendering and menu bar aliases.
- Implementation of the CLI plugin manager (`plugin install`, `plugin update`).
- Comprehensive documentation, examples, and a migration guide from SketchyBar.

---

### 5. Definition of Done (DoD)
- [ ] All checkboxes for the current Milestone are completed.
- [ ] Code passes `cargo clippy` and `cargo fmt` without warnings.
- [ ] Critical functionality (config parsing, IPC routing) is covered by unit tests.
- [ ] Documentation is updated and accurately reflects the implemented API.

