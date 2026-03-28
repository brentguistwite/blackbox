use anyhow::Context;
use chrono::{Datelike, Local, NaiveDate};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::collections::BTreeMap;

const DAY_LABEL_WIDTH: u16 = 4;
const CELL_WIDTH: u16 = 2;
const DAY_LABELS: [&str; 7] = ["Mon ", "    ", "Wed ", "    ", "Fri ", "    ", "Sun "];

/// Summary statistics derived from heatmap data.
pub struct HeatmapStats {
    pub total_commits: u32,
    pub active_days: u32,
    pub longest_streak: u32,
    pub current_streak: u32,
}

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

    /// Compute summary statistics: totals, active days, longest/current streaks.
    pub fn stats(&self) -> HeatmapStats {
        let total_commits: u32 = self.days.values().sum();
        let active_days: u32 = self.days.values().filter(|&&c| c > 0).count() as u32;
        let longest = longest_streak(&self.days);
        let current = current_streak(&self.days, Local::now().date_naive());
        HeatmapStats {
            total_commits,
            active_days,
            longest_streak: longest,
            current_streak: current,
        }
    }
}

/// Compute current streak ending on `today` (or yesterday if today has 0 commits).
fn current_streak(days: &BTreeMap<NaiveDate, u32>, today: NaiveDate) -> u32 {
    let start = if days.get(&today).copied().unwrap_or(0) > 0 {
        today
    } else {
        today - chrono::Duration::days(1)
    };
    let mut streak = 0u32;
    let mut date = start;
    loop {
        if days.get(&date).copied().unwrap_or(0) > 0 {
            streak += 1;
            date -= chrono::Duration::days(1);
        } else {
            break;
        }
    }
    streak
}

/// Map intensity tier (0-4) to ratatui Color.
fn intensity_color(tier: u8) -> Color {
    match tier {
        0 => Color::DarkGray,
        1 => Color::Rgb(0, 68, 0),
        2 => Color::Rgb(0, 128, 0),
        3 => Color::Rgb(0, 185, 0),
        _ => Color::Rgb(57, 211, 83),
    }
}

/// 2-char month abbreviation aligned to cell width.
fn month_label(month: u32) -> &'static str {
    match month {
        1 => "Ja",
        2 => "Fe",
        3 => "Mr",
        4 => "Ap",
        5 => "My",
        6 => "Jn",
        7 => "Jl",
        8 => "Au",
        9 => "Se",
        10 => "Oc",
        11 => "Nv",
        12 => "De",
        _ => "  ",
    }
}

/// Render GitHub-style contribution heatmap into a ratatui frame.
///
/// Columns = weeks (left=oldest), rows = 7 days (Mon at top).
/// Each cell is `██` (2 chars wide) colored by intensity tier.
/// Month labels appear above the first column of each new month.
/// Day-of-week labels (Mon/Wed/Fri) on the left.
/// Truncates weeks to fit when terminal is narrower than the grid.
pub fn render_heatmap(frame: &mut Frame, area: Rect, data: &HeatmapData, weeks: u32) {
    let available = area.width.saturating_sub(DAY_LABEL_WIDTH);
    let max_weeks = (available / CELL_WIDTH) as u32;
    let display_weeks = weeks.min(max_weeks);

    if display_weeks == 0 || area.height < 2 {
        return;
    }

    let today = Local::now().date_naive();
    let days_since_monday = today.weekday().num_days_from_monday();
    let this_monday = today - chrono::Duration::days(days_since_monday as i64);
    let start_monday = this_monday - chrono::Duration::weeks((display_weeks - 1) as i64);

    let mut lines: Vec<Line> = Vec::new();

    // Month labels row
    let mut month_spans: Vec<Span> = Vec::new();
    month_spans.push(Span::raw("    ")); // day label padding
    let mut prev_month: Option<u32> = None;
    for w in 0..display_weeks {
        let week_monday = start_monday + chrono::Duration::weeks(w as i64);
        let month = week_monday.month();
        if prev_month != Some(month) {
            month_spans.push(Span::styled(
                month_label(month).to_string(),
                Style::default().fg(Color::White),
            ));
        } else {
            month_spans.push(Span::raw("  "));
        }
        prev_month = Some(month);
    }
    lines.push(Line::from(month_spans));

    // Day rows: Mon(0) through Sun(6)
    for row in 0..7u32 {
        if (lines.len() as u16) >= area.height {
            break;
        }
        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::styled(
            DAY_LABELS[row as usize].to_string(),
            Style::default().fg(Color::DarkGray),
        ));
        for w in 0..display_weeks {
            let date = start_monday
                + chrono::Duration::weeks(w as i64)
                + chrono::Duration::days(row as i64);
            if date > today {
                // Future date: blank
                spans.push(Span::raw("  "));
            } else {
                let tier = data.intensity(date);
                spans.push(Span::styled(
                    "██",
                    Style::default().fg(intensity_color(tier)),
                ));
            }
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// Compute longest streak of consecutive days with >= 1 commit.
fn longest_streak(days: &BTreeMap<NaiveDate, u32>) -> u32 {
    let mut best = 0u32;
    let mut current = 0u32;
    let mut prev: Option<NaiveDate> = None;
    for (&date, &count) in days {
        if count == 0 {
            current = 0;
            prev = Some(date);
            continue;
        }
        if let Some(p) = prev {
            if date == p + chrono::Duration::days(1) {
                // Only extend if prev day also had commits
                if days.get(&p).copied().unwrap_or(0) > 0 {
                    current += 1;
                } else {
                    current = 1;
                }
            } else {
                current = 1;
            }
        } else {
            current = 1;
        }
        best = best.max(current);
        prev = Some(date);
    }
    best
}

/// Render the heatmap grid and summary to terminal via ratatui, then exit.
///
/// Loads config + DB, queries daily commit counts for the given week range,
/// renders a GitHub-style contribution grid with summary stats, then returns.
pub fn run_heatmap(weeks: u32) -> anyhow::Result<()> {
    let data_dir = crate::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = crate::db::open_db(&db_path)
        .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;

    let (from, to) = crate::query::heatmap_range(weeks);
    let counts = crate::query::query_daily_commit_counts(&conn, from, to)?;
    let data = HeatmapData::from_counts(counts);

    let stats = data.stats();

    // Render via ratatui raw mode, then exit
    let mut terminal = ratatui::init();
    terminal.draw(|frame| {
        let area = frame.area();
        // Reserve last row for summary
        let grid_area = Rect::new(area.x, area.y, area.width, area.height.saturating_sub(1));
        render_heatmap(frame, grid_area, &data, weeks);

        // Summary line at bottom
        let summary_text = format!(
            "  {} commits  {} active days  {} day streak",
            stats.total_commits, stats.active_days, stats.longest_streak
        );
        let summary_line = Line::from(Span::styled(
            summary_text,
            Style::default().fg(Color::White),
        ));
        let summary_area = Rect::new(area.x, area.y + grid_area.height, area.width, 1);
        frame.render_widget(Paragraph::new(vec![summary_line]), summary_area);
    })?;
    ratatui::restore();

    Ok(())
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

    #[test]
    fn render_heatmap_empty_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(120, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let data = HeatmapData::from_counts(BTreeMap::new());
        terminal
            .draw(|frame| {
                render_heatmap(frame, frame.area(), &data, 52);
            })
            .unwrap();
    }

    #[test]
    fn render_heatmap_narrow_terminal_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(10, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let data = HeatmapData::from_counts(BTreeMap::new());
        terminal
            .draw(|frame| {
                render_heatmap(frame, frame.area(), &data, 52);
            })
            .unwrap();
    }

    #[test]
    fn render_heatmap_with_data_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(120, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut counts = BTreeMap::new();
        let today = Local::now().date_naive();
        counts.insert(today, 5);
        counts.insert(today - chrono::Duration::days(1), 3);
        counts.insert(today - chrono::Duration::days(7), 10);
        let data = HeatmapData::from_counts(counts);
        terminal
            .draw(|frame| {
                render_heatmap(frame, frame.area(), &data, 52);
            })
            .unwrap();
    }

    #[test]
    fn render_heatmap_zero_height_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(120, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let data = HeatmapData::from_counts(BTreeMap::new());
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 120, 1);
                render_heatmap(frame, area, &data, 52);
            })
            .unwrap();
    }

    #[test]
    fn intensity_color_maps_all_tiers() {
        assert_eq!(intensity_color(0), Color::DarkGray);
        assert_eq!(intensity_color(1), Color::Rgb(0, 68, 0));
        assert_eq!(intensity_color(2), Color::Rgb(0, 128, 0));
        assert_eq!(intensity_color(3), Color::Rgb(0, 185, 0));
        assert_eq!(intensity_color(4), Color::Rgb(57, 211, 83));
    }

    #[test]
    fn stats_empty_data_all_zeros() {
        let data = HeatmapData::from_counts(BTreeMap::new());
        let s = data.stats();
        assert_eq!(s.total_commits, 0);
        assert_eq!(s.active_days, 0);
        assert_eq!(s.longest_streak, 0);
        assert_eq!(s.current_streak, 0);
    }

    #[test]
    fn stats_three_consecutive_days_streak() {
        let today = Local::now().date_naive();
        let mut counts = BTreeMap::new();
        counts.insert(today, 2);
        counts.insert(today - chrono::Duration::days(1), 3);
        counts.insert(today - chrono::Duration::days(2), 1);
        let data = HeatmapData::from_counts(counts);
        let s = data.stats();
        assert_eq!(s.total_commits, 6);
        assert_eq!(s.active_days, 3);
        assert_eq!(s.longest_streak, 3);
        assert_eq!(s.current_streak, 3);
    }

    #[test]
    fn stats_streak_resets_on_gap() {
        let today = Local::now().date_naive();
        let mut counts = BTreeMap::new();
        // 2-day streak ending today
        counts.insert(today, 1);
        counts.insert(today - chrono::Duration::days(1), 1);
        // gap on day -2
        counts.insert(today - chrono::Duration::days(2), 0);
        // 3-day streak earlier
        counts.insert(today - chrono::Duration::days(3), 1);
        counts.insert(today - chrono::Duration::days(4), 1);
        counts.insert(today - chrono::Duration::days(5), 1);
        let data = HeatmapData::from_counts(counts);
        let s = data.stats();
        assert_eq!(s.longest_streak, 3);
        assert_eq!(s.current_streak, 2);
    }

    #[test]
    fn stats_current_streak_from_yesterday_when_today_zero() {
        let today = Local::now().date_naive();
        let mut counts = BTreeMap::new();
        counts.insert(today, 0);
        counts.insert(today - chrono::Duration::days(1), 1);
        counts.insert(today - chrono::Duration::days(2), 1);
        let data = HeatmapData::from_counts(counts);
        let s = data.stats();
        assert_eq!(s.current_streak, 2);
    }

    #[test]
    fn longest_streak_helper_basic() {
        let mut days = BTreeMap::new();
        let base = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
        days.insert(base, 1);
        days.insert(base + chrono::Duration::days(1), 2);
        days.insert(base + chrono::Duration::days(2), 1);
        // gap
        days.insert(base + chrono::Duration::days(4), 3);
        assert_eq!(longest_streak(&days), 3);
    }
}
