use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default per-job cap. Chosen to cover a single cheap image (low-cost catalog
/// models estimate in the single digits to low tens of credits) while refusing
/// any video job until the user deliberately raises it in the settings file.
pub const DEFAULT_MAX_CREDITS_PER_JOB: u64 = 10;

pub const SETTINGS_FILE: &str = "mcp-bridge-settings.json";
pub const CONNECTION_FILE: &str = "mcp-bridge.json";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct BridgeSettings {
  /// Maximum estimated credits a single bridge job may cost. Requests can
  /// only lower this, never raise it. Edit this file by hand to raise it.
  pub max_credits_per_job: u64,
}

impl Default for BridgeSettings {
  fn default() -> Self {
    Self { max_credits_per_job: DEFAULT_MAX_CREDITS_PER_JOB }
  }
}

pub fn settings_path(state_dir: &Path) -> PathBuf {
  state_dir.join(SETTINGS_FILE)
}

/// Loads settings, creating the file with defaults if missing. A corrupt file
/// falls back to the (low) default rather than "unlimited".
pub fn load_or_create(state_dir: &Path) -> BridgeSettings {
  let path = settings_path(state_dir);
  match std::fs::read_to_string(&path) {
    Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
    Err(_) => {
      let settings = BridgeSettings::default();
      if let Ok(text) = serde_json::to_string_pretty(&settings) {
        let _ = write_private(&path, text.as_bytes());
      }
      settings
    }
  }
}

/// Effective cap for one request: the lower of the settings cap and the request cap.
pub fn effective_cap(settings_cap: u64, requested: Option<u64>) -> u64 {
  match requested {
    Some(r) => r.min(settings_cap),
    None => settings_cap,
  }
}

/// Writes a file readable only by the current user (0600 on unix).
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
  use std::io::Write;
  let tmp = path.with_extension("tmp");
  let _ = std::fs::remove_file(&tmp);
  {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
      use std::os::unix::fs::OpenOptionsExt;
      opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
  }
  std::fs::rename(&tmp, path)?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn request_can_lower_but_not_raise_cap() {
    assert_eq!(effective_cap(10, None), 10);
    assert_eq!(effective_cap(10, Some(3)), 3);
    assert_eq!(effective_cap(10, Some(500)), 10);
    assert_eq!(effective_cap(10, Some(0)), 0);
  }

  #[test]
  fn missing_or_corrupt_settings_use_low_default() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load_or_create(dir.path()).max_credits_per_job, DEFAULT_MAX_CREDITS_PER_JOB);
    assert!(settings_path(dir.path()).exists());
    std::fs::write(settings_path(dir.path()), "{not json").unwrap();
    assert_eq!(load_or_create(dir.path()).max_credits_per_job, DEFAULT_MAX_CREDITS_PER_JOB);
    std::fs::write(settings_path(dir.path()), r#"{"max_credits_per_job": 40}"#).unwrap();
    assert_eq!(load_or_create(dir.path()).max_credits_per_job, 40);
  }

  #[cfg(unix)]
  #[test]
  fn private_files_are_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(CONNECTION_FILE);
    write_private(&p, b"{}").unwrap();
    write_private(&p, b"{\"x\":1}").unwrap();
    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
  }
}
