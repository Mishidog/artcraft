# Building and running ArtCraft with the MCP bridge on macOS (Apple Silicon)

Tested design target: macOS 26, arm64, Node 22, git. These steps build a
**separate local copy** of ArtCraft; the installed ArtCraft 0.41.0 is not modified.
Quit the installed ArtCraft before launching this build (they share `~/Artcraft`).

## 1. One-time prerequisites

```bash
# Xcode command line tools (compiler, linker, git)
xcode-select --install          # skip if already installed

# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustup default stable
rustc --version                 # needs >= 1.85 (workspace uses edition 2024)

# Tauri CLI (v2)
cargo install tauri-cli --version "^2" --locked
```

## 2. Get the code

```bash
mkdir -p ~/src && cd ~/src
git clone https://github.com/Mishidog/artcraft.git artcraft-mcp
cd artcraft-mcp
git checkout mcp-bridge
```

## 3. Build the app with the bridge feature

```bash
cd ~/src/artcraft-mcp
(cd frontend && npm install)

export TAURI_FRONTEND_PATH="$PWD/frontend"
export TAURI_APP_PATH="$PWD/crates/desktop/artcraft"
export VITE_ENVIRONMENT_TYPE=production
export SQLX_OFFLINE=true

cargo tauri build --config crates/desktop/artcraft/tauri.conf.json \
  --features mcp-bridge --bundles app
```

The first build takes a while (roughly 15-40 min). The result is:
`target/release/bundle/macos/ArtCraft.app` (unsigned, local use only).

Quick sanity check without a full bundle (optional):
`SQLX_OFFLINE=true cargo check -p artcraft --features mcp-bridge`

## 4. Launch with the bridge turned on

The bridge only starts when the feature is compiled in **and**
`ARTCRAFT_MCP_BRIDGE=1` is set at launch:

```bash
ARTCRAFT_MCP_BRIDGE=1 ~/src/artcraft-mcp/target/release/bundle/macos/ArtCraft.app/Contents/MacOS/ArtCraft
```

(If macOS blocks the unsigned app, right-click ArtCraft.app > Open once, or run
`xattr -dr com.apple.quarantine ~/src/artcraft-mcp/target/release/bundle/macos/ArtCraft.app`.)

Log in inside the app as usual. Then check:

```bash
ls -l ~/Artcraft/state/mcp-bridge.json        # -rw------- {port, token}
cat  ~/Artcraft/state/mcp-bridge-settings.json # {"max_credits_per_job": 10}
```

To allow pricier jobs (e.g. a video), edit `max_credits_per_job` in
`~/Artcraft/state/mcp-bridge-settings.json` (read on every request; no restart needed).
Requests can only lower this cap, never raise it.

## 5. Build the MCP server

```bash
cd ~/src/artcraft-mcp/mcp/artcraft-mcp
npm install
npm run build
npm test        # optional: runs against a mock bridge, spends nothing
```

## 6. Register it with an MCP client (stdio)

Command: `node`, args: `["/Users/<you>/src/artcraft-mcp/mcp/artcraft-mcp/dist/index.js"]`

Claude Desktop / Cursor style config (`claude_desktop_config.json` or `~/.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "artcraft": {
      "command": "node",
      "args": ["/Users/<you>/src/artcraft-mcp/mcp/artcraft-mcp/dist/index.js"]
    }
  }
}
```

Claude Code: `claude mcp add artcraft -- node ~/src/artcraft-mcp/mcp/artcraft-mcp/dist/index.js`

The server reads `~/Artcraft/state/mcp-bridge.json` on every call, so it
keeps working after ArtCraft restarts (new port/token). Override the path with
`ARTCRAFT_MCP_BRIDGE_FILE` if needed.

## 7. First test (free)

Ask the client to call `artcraft_status`, then `list_models` (image), then
`estimate_cost`. None of these spend credits. `generate_image` / `generate_video`
spend credits, require `confirm: true`, and are refused if the estimate exceeds the cap.
