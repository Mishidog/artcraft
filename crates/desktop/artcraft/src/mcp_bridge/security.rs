use std::net::SocketAddr;

/// Why a request was refused before reaching a handler.
#[derive(Debug, PartialEq, Eq)]
pub enum Rejection {
  NotLoopback,
  OriginPresent,
  BadHost,
  MissingToken,
  BadToken,
}

impl Rejection {
  pub fn status(&self) -> u16 {
    match self {
      Rejection::MissingToken | Rejection::BadToken => 401,
      _ => 403,
    }
  }
  pub fn message(&self) -> &'static str {
    match self {
      Rejection::NotLoopback => "only loopback connections are allowed",
      Rejection::OriginPresent => "browser requests (Origin header) are not allowed",
      Rejection::BadHost => "Host header must be a loopback address",
      Rejection::MissingToken => "missing Authorization: Bearer token",
      Rejection::BadToken => "invalid bearer token",
    }
  }
}

/// Generates a random 32-byte token, hex encoded (64 chars).
pub fn generate_token() -> String {
  use rand::RngCore;
  let mut bytes = [0u8; 32];
  rand::rng().fill_bytes(&mut bytes);
  bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Constant-time comparison so token checks don't leak timing.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
  if a.len() != b.len() {
    return false;
  }
  a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn host_is_loopback(host: &str) -> bool {
  // Strip port; handle bracketed IPv6.
  let name = if let Some(rest) = host.strip_prefix('[') {
    rest.split(']').next().unwrap_or("")
  } else {
    host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host)
  };
  matches!(name.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1")
}

/// Checks every request. Pure function so it can be unit tested.
pub fn check_request(
  peer: SocketAddr,
  origin: Option<&str>,
  host: Option<&str>,
  authorization: Option<&str>,
  expected_token: &str,
) -> Result<(), Rejection> {
  if !peer.ip().is_loopback() {
    return Err(Rejection::NotLoopback);
  }
  if origin.is_some() {
    return Err(Rejection::OriginPresent);
  }
  match host {
    Some(h) if host_is_loopback(h) => {}
    _ => return Err(Rejection::BadHost),
  }
  let header = authorization.ok_or(Rejection::MissingToken)?;
  let token = header
    .strip_prefix("Bearer ")
    .or_else(|| header.strip_prefix("bearer "))
    .ok_or(Rejection::MissingToken)?
    .trim();
  if expected_token.is_empty() || !constant_time_eq(token.as_bytes(), expected_token.as_bytes()) {
    return Err(Rejection::BadToken);
  }
  Ok(())
}

/// Media tokens are embedded in URLs and file names; keep them boring.
pub fn is_safe_media_token(token: &str) -> bool {
  !token.is_empty() && token.len() <= 128 && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// File extension taken from a CDN URL path, limited to short alphanumerics.
pub fn safe_extension(url_path: &str) -> String {
  let last = url_path.rsplit('/').next().unwrap_or("");
  match last.rsplit_once('.') {
    Some((_, ext)) if !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) => ext.to_ascii_lowercase(),
    _ => "bin".to_string(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const TOKEN: &str = "abc123";
  fn lo() -> SocketAddr { "127.0.0.1:5555".parse().unwrap() }

  #[test]
  fn accepts_valid_request() {
    assert_eq!(check_request(lo(), None, Some("127.0.0.1:4000"), Some("Bearer abc123"), TOKEN), Ok(()));
    assert_eq!(check_request("[::1]:9".parse().unwrap(), None, Some("[::1]:4000"), Some("Bearer abc123"), TOKEN), Ok(()));
    assert_eq!(check_request(lo(), None, Some("localhost:4000"), Some("Bearer abc123"), TOKEN), Ok(()));
  }

  #[test]
  fn rejects_missing_or_wrong_token() {
    assert_eq!(check_request(lo(), None, Some("127.0.0.1"), None, TOKEN), Err(Rejection::MissingToken));
    assert_eq!(check_request(lo(), None, Some("127.0.0.1"), Some("Basic abc123"), TOKEN), Err(Rejection::MissingToken));
    assert_eq!(check_request(lo(), None, Some("127.0.0.1"), Some("Bearer abc124"), TOKEN), Err(Rejection::BadToken));
    assert_eq!(check_request(lo(), None, Some("127.0.0.1"), Some("Bearer abc12"), TOKEN), Err(Rejection::BadToken));
    assert_eq!(check_request(lo(), None, Some("127.0.0.1"), Some("Bearer "), ""), Err(Rejection::BadToken));
  }

  #[test]
  fn rejects_any_origin_header() {
    assert_eq!(check_request(lo(), Some("https://evil.example"), Some("127.0.0.1"), Some("Bearer abc123"), TOKEN), Err(Rejection::OriginPresent));
    assert_eq!(check_request(lo(), Some("null"), Some("127.0.0.1"), Some("Bearer abc123"), TOKEN), Err(Rejection::OriginPresent));
  }

  #[test]
  fn rejects_non_loopback_and_rebinding() {
    assert_eq!(check_request("192.168.1.5:1".parse().unwrap(), None, Some("127.0.0.1"), Some("Bearer abc123"), TOKEN), Err(Rejection::NotLoopback));
    assert_eq!(check_request(lo(), None, Some("evil.example:4000"), Some("Bearer abc123"), TOKEN), Err(Rejection::BadHost));
    assert_eq!(check_request(lo(), None, None, Some("Bearer abc123"), TOKEN), Err(Rejection::BadHost));
  }

  #[test]
  fn token_is_32_random_bytes() {
    let a = generate_token();
    assert_eq!(a.len(), 64);
    assert_ne!(a, generate_token());
  }

  #[test]
  fn media_tokens_and_extensions_are_sanitized() {
    assert!(is_safe_media_token("m_abc123"));
    assert!(!is_safe_media_token("../etc/passwd"));
    assert!(!is_safe_media_token(""));
    assert_eq!(safe_extension("/media/a/b/file.MP4"), "mp4");
    assert_eq!(safe_extension("/media/a/b/file"), "bin");
    assert_eq!(safe_extension("/x/y.p/ng"), "bin");
  }
}
