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
