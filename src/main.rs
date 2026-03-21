use anyhow::Context;
use chrono::{DateTime, Utc};
use clap::{CommandFactory, Parser};
use blackbox::cli::{Cli, Commands};
use blackbox::output::OutputFormat;
use blackbox::query::ActivitySummary;

fn run_query(
    period_label: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    format: OutputFormat,
    summarize: bool,
) -> anyhow::Result<()> {
    let config = blackbox::config::load_config()?;
    let data_dir = blackbox::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = blackbox::db::open_db(&db_path)
        .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
    let mut repos = blackbox::query::query_activity(
        &conn,
        from,
        to,
        config.session_gap_minutes,
        config.first_commit_minutes,
    )?;

    blackbox::enrichment::enrich_with_prs(&mut repos);

    let total_commits: usize = repos.iter().map(|r| r.commits).sum();
    let total_reviews: usize = repos.iter().map(|r| {
        r.reviews.iter().map(|rv| rv.pr_number).collect::<std::collections::BTreeSet<_>>().len()
    }).sum();
    let total_time = blackbox::query::global_estimated_time(&repos, config.session_gap_minutes, config.first_commit_minutes);
    let total_ai_session_time = repos.iter().fold(chrono::Duration::zero(), |acc, r| {
        acc + r.ai_sessions.iter().fold(chrono::Duration::zero(), |a, s| a + s.duration)
    });

    let summary = ActivitySummary {
        period_label: period_label.to_string(),
        total_commits,
        total_reviews,
        total_repos: repos.len(),
        total_estimated_time: total_time,
        total_ai_session_time,
        repos,
    };

    if summarize {
        let llm_config = blackbox::llm::build_llm_config(&config)?;
        let json = blackbox::output::render_json(&summary);
        blackbox::llm::summarize_activity(&llm_config, &json)?;
    } else {
        match format {
            OutputFormat::Pretty => blackbox::output::render_summary(&summary),
            OutputFormat::Json => println!("{}", blackbox::output::render_json(&summary)),
            OutputFormat::Csv => println!("{}", blackbox::output::render_csv(&summary)),
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // First-run detection: redirect to setup wizard if no config exists
    if !blackbox::config::config_exists() && !cli.command.is_exempt_from_config_check() {
        println!("Welcome to blackbox! No config found. Let's get you set up.\n");
        match blackbox::setup::run_setup() {
            Ok(()) => {
                println!("\nYou're all set! Run your command again to get started.");
                return Ok(());
            }
            Err(_) => {
                println!("\nNo worries! Set up manually anytime with: blackbox setup");
                return Ok(());
            }
        }
    }

    match cli.command {
        Commands::Init { watch_dirs, poll_interval } => {
            blackbox::config::run_init(watch_dirs, poll_interval)?;
        }
        Commands::Start => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            blackbox::daemon::start_daemon(config, &data_dir)?;
        }
        Commands::Stop => {
            let data_dir = blackbox::config::data_dir()?;
            blackbox::daemon::stop_daemon(&data_dir)?;
        }
        Commands::Status => {
            let data_dir = blackbox::config::data_dir()?;
            blackbox::daemon::daemon_status(&data_dir)?;
        }
        Commands::Today { format, summarize } => {
            let (from, to) = blackbox::query::today_range();
            run_query("Today", from, to, format, summarize)?;
        }
        Commands::Week { format, summarize } => {
            let (from, to) = blackbox::query::week_range();
            run_query("This Week", from, to, format, summarize)?;
        }
        Commands::Month { format, summarize } => {
            let (from, to) = blackbox::query::month_range();
            run_query("This Month", from, to, format, summarize)?;
        }
        Commands::Yesterday { format, summarize } => {
            let (from, to) = blackbox::query::yesterday_range();
            run_query("Yesterday", from, to, format, summarize)?;
        }
        Commands::Query { from, to, format, summarize } => {
            let (range_from, range_to) = blackbox::query::custom_range(&from, &to)?;
            run_query(&format!("{} to {}", from, to), range_from, range_to, format, summarize)?;
        }
        Commands::Rhythms { range, format } => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)
                .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
            let (from, to) = range.to_range();
            let repos = blackbox::query::query_activity(
                &conn, from, to, config.session_gap_minutes, config.first_commit_minutes,
            )?;
            let hourly = blackbox::insights::hourly_distribution(&repos);
            let weekly = blackbox::insights::weekly_rhythm(&repos);
            match format {
                OutputFormat::Json => {
                    let obj = serde_json::json!({
                        "hourly": hourly.to_vec(),
                        "weekly": weekly.to_vec(),
                    });
                    println!("{}", serde_json::to_string_pretty(&obj).unwrap());
                }
                _ => {
                    println!("{}", blackbox::output::render_rhythms(&hourly, &weekly));
                }
            }
        }
        Commands::Heatmap { weeks } => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)
                .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
            // Query enough history to cover the requested weeks
            let now = chrono::Utc::now();
            let from = now - chrono::Duration::weeks(weeks as i64 + 1);
            let repos = blackbox::query::query_activity(
                &conn, from, now, config.session_gap_minutes, config.first_commit_minutes,
            )?;
            let counts = blackbox::insights::daily_commit_counts(&repos);
            println!("{}", blackbox::output::render_heatmap(&counts, weeks));
        }
        Commands::Streak => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)
                .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
            let epoch = chrono::TimeZone::timestamp_opt(&chrono::Utc, 0, 0).single().expect("epoch");
            let now = chrono::Utc::now();
            let repos = blackbox::query::query_activity(
                &conn, epoch, now, config.session_gap_minutes, config.first_commit_minutes,
            )?;
            let as_of = chrono::Local::now().date_naive();
            let info = blackbox::insights::streak_info(&repos, &config.streak_rest_days, as_of);
            println!("{}", blackbox::output::render_streak(&info));
        }
        Commands::Install => {
            blackbox::service::install()?;
        }
        Commands::Uninstall => {
            blackbox::service::uninstall()?;
        }
        Commands::RunForeground => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            blackbox::daemon::run_foreground(config, &data_dir)?;
        }
        Commands::Hook { shell } => {
            let script = blackbox::shell_hook::generate_hook(&shell)?;
            print!("{}", script);
        }
        Commands::NotifyDir { path } => {
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)?;
            blackbox::db::record_directory_presence(&conn, &path)?;
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            clap_complete::aot::generate(shell, &mut cmd, "blackbox", &mut std::io::stdout());
        }
        Commands::Doctor => {
            let all_ok = blackbox::doctor::run_doctor()?;
            if !all_ok {
                std::process::exit(1);
            }
        }
        Commands::Setup => {
            blackbox::setup::run_setup()?;
        }
        Commands::Live => {
            blackbox::tui::run_live()?;
        }
        Commands::Standup { week, summarize, webhook } => {
            let label;
            let range_fn: fn() -> (DateTime<Utc>, DateTime<Utc>);
            if week {
                label = "This Week";
                range_fn = blackbox::query::week_range;
            } else {
                label = "Today";
                range_fn = blackbox::query::today_range;
            }
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)
                .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
            let (from, to) = range_fn();
            let mut repos = blackbox::query::query_activity(
                &conn, from, to, config.session_gap_minutes, config.first_commit_minutes,
            )?;
            blackbox::enrichment::enrich_with_prs(&mut repos);
            let total_commits: usize = repos.iter().map(|r| r.commits).sum();
            let total_reviews: usize = repos.iter().map(|r| {
        r.reviews.iter().map(|rv| rv.pr_number).collect::<std::collections::BTreeSet<_>>().len()
    }).sum();
            let total_time = blackbox::query::global_estimated_time(&repos, config.session_gap_minutes, config.first_commit_minutes);
            let total_ai_session_time = repos.iter().fold(chrono::Duration::zero(), |acc, r| {
                acc + r.ai_sessions.iter().fold(chrono::Duration::zero(), |a, s| a + s.duration)
            });
            let summary = ActivitySummary {
                period_label: label.to_string(),
                total_commits,
                total_reviews,
                total_repos: repos.len(),
                total_estimated_time: total_time,
                total_ai_session_time,
                repos,
            };
            if summarize {
                let llm_config = blackbox::llm::build_llm_config(&config)?;
                let json = blackbox::output::render_json(&summary);
                blackbox::llm::summarize_activity(&llm_config, &json)?;
            } else {
                let standup_text = blackbox::output::render_standup(&summary);
                println!("{}", standup_text);

                // Webhook: CLI flag overrides config value
                let webhook_url = webhook.or(config.standup_webhook_url);
                if let Some(url) = webhook_url {
                    blackbox::webhook::post_to_webhook(&url, &standup_text);
                }
            }
        }
    }

    Ok(())
}
