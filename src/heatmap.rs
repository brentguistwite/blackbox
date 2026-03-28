use chrono::NaiveDate;
use std::collections::BTreeMap;

/// Holds daily commit counts and precomputed max for intensity calculation.
pub struct HeatmapData {
    pub days: BTreeMap<NaiveDate, u32>,
    pub max_count: u32,
}

impl HeatmapData {
    /// Build HeatmapData from a daily counts map, computing max_count.
    pub fn from_counts(counts: BTreeMap<NaiveDate, u32>) -> Self {
        let max_count = counts.values().copied().max().unwrap_or(0);
        Self {
            days: counts,
            max_count,
        }
    }

    /// Return intensity tier 0-4 for a given date.
    /// 0=no commits, 1=low, 2=medium, 3=high, 4=max.
    /// Thresholds: 1..=25%, 26..=50%, 51..=75%, 76..=100% of max_count.
    pub fn intensity(&self, date: NaiveDate) -> u8 {
        if self.max_count == 0 {
            return 0;
        }
        let count = self.days.get(&date).copied().unwrap_or(0);
        if count == 0 {
            return 0;
        }
        let quarter = self.max_count / 4;
        if quarter == 0 {
            // max_count is 1-3: any nonzero count gets tier based on ratio
            // but since count > 0 and max is small, map proportionally
            if count >= self.max_count {
                return 4;
            }
            // For max 2-3, intermediate values
            let ratio = (count as f64) / (self.max_count as f64);
            if ratio >= 0.75 {
                4
            } else if ratio >= 0.5 {
                3
            } else if ratio >= 0.25 {
                2
            } else {
                1
            }
        } else if count >= 3 * quarter {
            4
        } else if count >= 2 * quarter {
            3
        } else if count >= quarter {
            2
        } else {
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_map_all_intensity_zero() {
        let data = HeatmapData::from_counts(BTreeMap::new());
        assert_eq!(data.max_count, 0);
        let date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        assert_eq!(data.intensity(date), 0);
    }

    #[test]
    fn single_commit_intensity_nonzero() {
        let mut counts = BTreeMap::new();
        let date = NaiveDate::from_ymd_opt(2025, 6, 15).unwrap();
        counts.insert(date, 1);
        let data = HeatmapData::from_counts(counts);
        assert_eq!(data.max_count, 1);
        assert!(data.intensity(date) >= 1);
    }

    #[test]
    fn high_count_intensity_four() {
        let mut counts = BTreeMap::new();
        let d1 = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();
        let d3 = NaiveDate::from_ymd_opt(2025, 1, 3).unwrap();
        counts.insert(d1, 4);
        counts.insert(d2, 8);
        counts.insert(d3, 12);
        let data = HeatmapData::from_counts(counts);
        assert_eq!(data.max_count, 12);
        // 12 commits = max → intensity 4
        assert_eq!(data.intensity(d3), 4);
        // 4 commits = 4/12 ~33% → quarter=3, 4>=3 → tier 2
        assert_eq!(data.intensity(d1), 2);
    }

    #[test]
    fn missing_date_returns_zero() {
        let mut counts = BTreeMap::new();
        counts.insert(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(), 5);
        let data = HeatmapData::from_counts(counts);
        let missing = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();
        assert_eq!(data.intensity(missing), 0);
    }

    #[test]
    fn all_tiers_with_large_max() {
        let mut counts = BTreeMap::new();
        let base = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        // max = 100, quarter = 25
        counts.insert(base, 100);
        counts.insert(base.succ_opt().unwrap(), 0);   // tier 0
        counts.insert(NaiveDate::from_ymd_opt(2025, 1, 3).unwrap(), 10);  // tier 1 (10 < 25)
        counts.insert(NaiveDate::from_ymd_opt(2025, 1, 4).unwrap(), 30);  // tier 2 (30 >= 25)
        counts.insert(NaiveDate::from_ymd_opt(2025, 1, 5).unwrap(), 55);  // tier 3 (55 >= 50)
        counts.insert(NaiveDate::from_ymd_opt(2025, 1, 6).unwrap(), 80);  // tier 4 (80 >= 75)
        let data = HeatmapData::from_counts(counts);

        assert_eq!(data.intensity(base), 4);  // 100 = max
        assert_eq!(data.intensity(base.succ_opt().unwrap()), 0);
        assert_eq!(data.intensity(NaiveDate::from_ymd_opt(2025, 1, 3).unwrap()), 1);
        assert_eq!(data.intensity(NaiveDate::from_ymd_opt(2025, 1, 4).unwrap()), 2);
        assert_eq!(data.intensity(NaiveDate::from_ymd_opt(2025, 1, 5).unwrap()), 3);
        assert_eq!(data.intensity(NaiveDate::from_ymd_opt(2025, 1, 6).unwrap()), 4);
    }
}
