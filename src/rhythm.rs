use anyhow::Context;
use chrono::{Duration, Local, TimeZone, Utc};

use crate::output::{OutputFormat, RhythmReport};

/// Run the rhythm analysis command: query all metrics for the past N days, render output.
pub fn run_rhythm(days: u64, format: OutputFormat) -> anyhow::Result<()> {
    if days == 0 {
        anyhow::bail!("--days must be >= 1");
    }

    if matches!(format, OutputFormat::Csv) {
        anyhow::bail!("rhythm command does not support --format csv");
    }

    let config = crate::config::load_config()?;
    let data_dir = crate::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = crate::db::open_db(&db_path)
        .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;

    // Compute time range: past N days from local midnight today
    let local_today = Local::now().date_naive();
    let start_date = local_today - Duration::days(days as i64);
    let start_local = start_date.and_hms_opt(0, 0, 0).unwrap();
    let from = Local
        .from_local_datetime(&start_local)
        .unwrap()
        .with_timezone(&Utc);
    let to = Utc::now();

    let hour_histogram = crate::query::commit_hour_histogram(&conn, from, to)?;
    let dow_histogram = crate::query::commit_dow_histogram(&conn, from, to)?;
    let after_hours = crate::query::after_hours_ratio(&conn, from, to)?;
    let session_distribution = crate::query::session_length_distribution(
        &conn,
        from,
        to,
        config.session_gap_minutes,
        config.first_commit_minutes,
    )?;
    let burst_stats = crate::query::burst_pattern(&conn, from, to)?;

    let report = RhythmReport {
        days,
        hour_histogram,
        dow_histogram,
        after_hours,
        session_distribution,
        burst_stats,
    };

    match format {
        OutputFormat::Pretty => println!("{}", crate::output::render_rhythm(&report)),
        OutputFormat::Json => println!("{}", crate::output::render_rhythm_json(&report)),
        OutputFormat::Csv => unreachable!(),
    }

    Ok(())
}
