use serde_json::{Map, Value};

/// Fields a bridge client may never set: the bridge always bills through the
/// signed-in ArtCraft account and never impersonates frontend callers.
pub const FORBIDDEN_FIELDS: &[&str] = &[
  "provider", "frontend_caller", "frontend_subscriber_id", "frontend_subscriber_payload", "estimate_only",
];

/// Fields that multiply a video estimate (the UI multiplies the unit cost by the count).
const VIDEO_COUNT_FIELDS: &[&str] = &["generation_count", "video_batch_count", "batch_size", "count"];

#[derive(Debug, PartialEq)]
pub enum CapDecision {
  Allowed { estimated_credits: u64 },
  /// Estimate response had no usable cost; we refuse rather than guess.
  UnknownCost,
  OverCap { estimated_credits: u64, cap: u64 },
}

/// Strips forbidden keys and the bridge-only `max_credits` key.
pub fn sanitize_fields(mut fields: Map<String, Value>) -> (Map<String, Value>, Option<u64>) {
  let requested_cap = fields.remove("max_credits").and_then(|v| v.as_u64());
  for key in FORBIDDEN_FIELDS {
    fields.remove(*key);
  }
  (fields, requested_cap)
}

pub fn video_multiplier(fields: &Map<String, Value>) -> u64 {
  VIDEO_COUNT_FIELDS.iter().filter_map(|k| fields.get(*k).and_then(Value::as_u64)).max().unwrap_or(1).max(1)
}

/// Reads `cost_in_credits` from an estimate payload and compares it to the cap.
pub fn decide(estimate_payload: &Value, multiplier: u64, cap: u64) -> CapDecision {
  // A blank cost alone is never treated as free. Only allow it when the estimator
  // explicitly confirms the job is free, succeeded, and reports no non-zero cost.
  if is_confirmed_free(estimate_payload) {
    return CapDecision::Allowed { estimated_credits: 0 };
  }
  let unit = match estimate_payload.get("cost_in_credits") {
    Some(Value::Number(n)) => n.as_f64(),
    _ => None,
  };
  let unit = match unit {
    Some(u) if u.is_finite() && u >= 0.0 => u.ceil() as u64,
    _ => return CapDecision::UnknownCost,
  };
  let estimated_credits = unit.saturating_mul(multiplier.max(1));
  if estimated_credits > cap {
    CapDecision::OverCap { estimated_credits, cap }
  } else {
    CapDecision::Allowed { estimated_credits }
  }
}

fn zero_or_null(v: Option<&Value>) -> bool {
  match v {
    None | Some(Value::Null) => true,
    Some(Value::Number(n)) => n.as_f64() == Some(0.0),
    _ => false,
  }
}

/// True only when the estimate explicitly says `is_free: true` and `success: true`,
/// isn't rate limited, and any reported credit or USD cost is zero or absent.
pub fn is_confirmed_free(p: &Value) -> bool {
  p.get("is_free") == Some(&Value::Bool(true))
    && p.get("success") == Some(&Value::Bool(true))
    && p.get("is_rate_limited") != Some(&Value::Bool(true))
    && zero_or_null(p.get("cost_in_credits"))
    && zero_or_null(p.get("cost_in_usd_cents"))
}

#[cfg(test)]
mod free_tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn confirmed_free_is_allowed() {
    let p = json!({"cost_in_credits": null, "cost_in_usd_cents": null, "is_free": true, "success": true, "is_rate_limited": false});
    assert_eq!(decide(&p, 1, 10), CapDecision::Allowed { estimated_credits: 0 });
    assert_eq!(decide(&p, 4, 0), CapDecision::Allowed { estimated_credits: 0 });
  }

  #[test]
  fn blank_cost_without_free_flag_is_refused() {
    assert_eq!(decide(&json!({"cost_in_credits": null, "success": true}), 1, 10), CapDecision::UnknownCost);
    assert_eq!(decide(&json!({"cost_in_credits": null, "is_free": false, "success": true}), 1, 10), CapDecision::UnknownCost);
    assert_eq!(decide(&json!({"cost_in_credits": null, "is_free": "true", "success": true}), 1, 10), CapDecision::UnknownCost);
    assert_eq!(decide(&json!({"cost_in_credits": null, "is_free": true}), 1, 10), CapDecision::UnknownCost);
    assert_eq!(decide(&json!({"cost_in_credits": null, "is_free": true, "success": true, "is_rate_limited": true}), 1, 10), CapDecision::UnknownCost);
  }

  #[test]
  fn free_flag_with_real_cost_uses_cost() {
    assert_eq!(decide(&json!({"cost_in_credits": 35, "is_free": true, "success": true}), 1, 10), CapDecision::OverCap { estimated_credits: 35, cap: 10 });
    assert_eq!(decide(&json!({"cost_in_credits": null, "cost_in_usd_cents": 35, "is_free": true, "success": true}), 1, 10), CapDecision::UnknownCost);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn allows_under_or_equal_cap() {
    assert_eq!(decide(&json!({"cost_in_credits": 4}), 1, 10), CapDecision::Allowed { estimated_credits: 4 });
    assert_eq!(decide(&json!({"cost_in_credits": 10}), 1, 10), CapDecision::Allowed { estimated_credits: 10 });
  }

  #[test]
  fn refuses_over_cap_and_counts_multiplier() {
    assert_eq!(decide(&json!({"cost_in_credits": 11}), 1, 10), CapDecision::OverCap { estimated_credits: 11, cap: 10 });
    assert_eq!(decide(&json!({"cost_in_credits": 4}), 3, 10), CapDecision::OverCap { estimated_credits: 12, cap: 10 });
    assert_eq!(decide(&json!({"cost_in_credits": 2.1}), 1, 2), CapDecision::OverCap { estimated_credits: 3, cap: 2 });
  }

  #[test]
  fn refuses_unknown_cost() {
    assert_eq!(decide(&json!({}), 1, 10), CapDecision::UnknownCost);
    assert_eq!(decide(&json!({"cost_in_credits": null}), 1, 10), CapDecision::UnknownCost);
    assert_eq!(decide(&json!({"cost_in_credits": "5"}), 1, 10), CapDecision::UnknownCost);
    assert_eq!(decide(&json!({"cost_in_credits": -1}), 1, 10), CapDecision::UnknownCost);
  }

  #[test]
  fn sanitize_removes_provider_and_reads_cap() {
    let fields = json!({"model":"m","prompt":"p","provider":"midjourney","frontend_caller":"x","max_credits":5}).as_object().unwrap().clone();
    let (clean, cap) = sanitize_fields(fields);
    assert_eq!(cap, Some(5));
    assert!(clean.get("provider").is_none() && clean.get("frontend_caller").is_none() && clean.get("max_credits").is_none());
    assert_eq!(clean.get("model"), Some(&json!("m")));
  }

  #[test]
  fn video_multiplier_uses_largest_count() {
    assert_eq!(video_multiplier(json!({}).as_object().unwrap()), 1);
    assert_eq!(video_multiplier(json!({"generation_count": 2, "batch_size": 4}).as_object().unwrap()), 4);
    assert_eq!(video_multiplier(json!({"count": 0}).as_object().unwrap()), 1);
  }
}
