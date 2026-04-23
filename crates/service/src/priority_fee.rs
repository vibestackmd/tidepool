//! Priority-fee percentile computation.
//!
//! Helius's `getPriorityFeeEstimate` returns either a single fee (for
//! a requested priority level) or a map of six percentiles. The math
//! is pure — a sorted list of recent prioritization fees in, a fee
//! number out. We keep it here, separate from any upstream or
//! transport plumbing, so it's trivially testable.
//!
//! Percentile mapping follows Helius's documented ladder:
//! Min (0th) < Low (25th) < Medium (50th) < High (75th) < VeryHigh (95th) < UnsafeMax (100th).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PriorityLevel {
    Min,
    Low,
    Medium,
    High,
    VeryHigh,
    UnsafeMax,
}

impl PriorityLevel {
    /// Percentile (0.0–100.0) in the sorted-fees distribution.
    #[must_use]
    pub fn percentile(self) -> f64 {
        match self {
            Self::Min => 0.0,
            Self::Low => 25.0,
            Self::Medium => 50.0,
            Self::High => 75.0,
            Self::VeryHigh => 95.0,
            Self::UnsafeMax => 100.0,
        }
    }
}

/// Per-level estimates; matches what Helius returns for
/// `includeAllPriorityFeeLevels: true`. Helius wire-serializes these
/// as floating-point even though the values are fundamentally
/// integer (micro-lamports-per-CU), so the deserialize side is `f64`
/// to keep round-trips happy. Our local computation still works
/// entirely in `u64` — serde handles the conversion at the wire
/// boundary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PriorityFeeLevels {
    pub min: f64,
    pub low: f64,
    pub medium: f64,
    pub high: f64,
    pub very_high: f64,
    pub unsafe_max: f64,
}

/// Compute all six percentiles over a slice of recent prioritization
/// fees in lamports-per-CU. Empty input yields all zeros — matching
/// the reality that a local validator with no contention (Surfpool)
/// has no fee data to percentile over.
#[must_use]
pub fn compute_levels(fees: &[u64]) -> PriorityFeeLevels {
    if fees.is_empty() {
        return PriorityFeeLevels::default();
    }
    let mut sorted = fees.to_vec();
    sorted.sort_unstable();
    PriorityFeeLevels {
        min: percentile_at(&sorted, PriorityLevel::Min) as f64,
        low: percentile_at(&sorted, PriorityLevel::Low) as f64,
        medium: percentile_at(&sorted, PriorityLevel::Medium) as f64,
        high: percentile_at(&sorted, PriorityLevel::High) as f64,
        very_high: percentile_at(&sorted, PriorityLevel::VeryHigh) as f64,
        unsafe_max: percentile_at(&sorted, PriorityLevel::UnsafeMax) as f64,
    }
}

/// Pull one level out of a pre-sorted fee distribution. Clamps to the
/// last element so UnsafeMax on a 1-element slice still returns that
/// element rather than wrapping.
#[must_use]
pub fn percentile_at(sorted: &[u64], level: PriorityLevel) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len();
    let p = level.percentile();
    let idx = ((p / 100.0) * (n as f64 - 1.0)).round() as usize;
    sorted[idx.min(n - 1)]
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // Fee levels are integers cast to f64; direct equality is intentional.
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_all_zeros() {
        let levels = compute_levels(&[]);
        assert_eq!(levels, PriorityFeeLevels::default());
    }

    #[test]
    fn single_fee_returns_that_fee_at_every_level() {
        let levels = compute_levels(&[1000]);
        assert_eq!(levels.min, 1000.0);
        assert_eq!(levels.low, 1000.0);
        assert_eq!(levels.medium, 1000.0);
        assert_eq!(levels.high, 1000.0);
        assert_eq!(levels.very_high, 1000.0);
        assert_eq!(levels.unsafe_max, 1000.0);
    }

    #[test]
    fn monotonic_ladder_across_distribution() {
        // Round-number fees so percentile positions are obvious.
        let fees: Vec<u64> = (1..=100).map(|i| i * 1000).collect();
        let levels = compute_levels(&fees);
        assert!(levels.min <= levels.low);
        assert!(levels.low <= levels.medium);
        assert!(levels.medium <= levels.high);
        assert!(levels.high <= levels.very_high);
        assert!(levels.very_high <= levels.unsafe_max);
        // idx = round(p/100 * (n-1)) for n=100: min→0, medium→50, max→99.
        assert_eq!(levels.min, 1_000.0);
        assert_eq!(levels.medium, 51_000.0);
        assert_eq!(levels.unsafe_max, 100_000.0);
    }

    #[test]
    fn input_order_doesnt_affect_output() {
        let ordered = vec![100u64, 200, 300, 400, 500, 600, 700, 800, 900, 1000];
        let shuffled = vec![500u64, 1000, 300, 700, 100, 900, 200, 800, 400, 600];
        assert_eq!(compute_levels(&ordered), compute_levels(&shuffled));
    }

    #[test]
    fn percentile_at_is_consistent_with_compute_levels() {
        let mut sorted = vec![10u64, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        sorted.sort_unstable();
        let levels = compute_levels(&sorted);
        assert_eq!(
            percentile_at(&sorted, PriorityLevel::Min) as f64,
            levels.min
        );
        assert_eq!(
            percentile_at(&sorted, PriorityLevel::Medium) as f64,
            levels.medium
        );
        assert_eq!(
            percentile_at(&sorted, PriorityLevel::UnsafeMax) as f64,
            levels.unsafe_max
        );
    }

    #[test]
    fn priority_level_serializes_camel_case() {
        let json = serde_json::to_string(&PriorityLevel::VeryHigh).unwrap();
        assert_eq!(json, "\"veryHigh\"");
        let json = serde_json::to_string(&PriorityLevel::UnsafeMax).unwrap();
        assert_eq!(json, "\"unsafeMax\"");
    }
}
