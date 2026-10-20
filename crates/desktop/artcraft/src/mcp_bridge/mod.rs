//! Optional local control server ("MCP bridge") so a local MCP helper can drive ArtCraft.
//!
//! Compiled only with `--features mcp-bridge` and started only when the
//! `ARTCRAFT_MCP_BRIDGE=1` environment variable is set at launch.
//!
//! Security model:
//!  - Binds 127.0.0.1 on a random port; non-loopback peers are rejected.
//!  - A random 32-byte bearer token is written with {port, token} to
//!    `~/Artcraft/state/mcp-bridge.json` (0600). Every request needs it.
//!  - Any `Origin` header is rejected (blocks browser CSRF), and the `Host`
//!    header must be a loopback name (blocks DNS rebinding).
//!  - Every generation is estimated first and refused if it exceeds the
//!    per-job credit cap in `~/Artcraft/state/mcp-bridge-settings.json`.
//!  - No delete, billing, login or credential endpoints exist.

pub mod security;
pub mod settings;
pub mod cost;
#[cfg(feature = "mcp-bridge")]
pub mod server;

pub const ENV_FLAG: &str = "ARTCRAFT_MCP_BRIDGE";

pub fn enabled_by_env() -> bool {
  matches!(std::env::var(ENV_FLAG).as_deref(), Ok("1") | Ok("true") | Ok("yes"))
}
