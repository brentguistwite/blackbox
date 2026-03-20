use anyhow::Context;
use blackbox::cli::{Cli, Commands};
use blackbox::output::OutputFormat;
use blackbox::query::ActivitySummary;
use chrono::{DateTime, Utc};
use clap::{CommandFactory, Parser};

fn build_summary(
    label: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    all_prs: bool,
) -> anyhow::Result<(blackbox::config::Config, ActivitySummary)> {
    let config = blackbox::config::load_config()?;
    let data_dir = blackbox::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = blackbox::db::open_db(&db_path)
        .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
    let mut repos = blackbox::query::query_activity(
        &conn, from, to, config.session_gap_minutes, config.first_commit_minutes,
    )?;
    if all_prs {
        blackbox::enrichment::enrich_with_all_prs(&mut repos);
    } else {
        blackbox::enrichment::enrich_with_prs(&mut repos);
    }
    let total_commits: usize = repos.iter().map(|r| r.commits).sum();
    let total_reviews: usize = repos
        .iter()
        .map(|r| {
            r.reviews
                .iter()
                .map(|rv| rv.pr_number)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        })
        .sum();
    let total_time = blackbox::query::global_estimated_time(
        &repos,
        config.session_gap_minutes,
        config.first_commit_minutes,
    );
    let total_ai_session_time = repos.iter().fold(chrono::Duration::zero(), |acc, r| {
        acc + r
            .ai_sessions
            .iter()
            .fold(chrono::Duration::zero(), |a, s| a + s.duration)
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
    Ok((config, summary))
}

fn run_query(
    period_label: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    format: OutputFormat,
    summarize: bool,
) -> anyhow::Result<()> {
    let (config, summary) = build_summary(period_label, from, to, false)?;

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
        Commands::Init {
            watch_dirs,
            poll_interval,
        } => {
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
        Commands::Tickets { range, format } => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)
                .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
            let (from, to) = range.to_range();
            let repos = blackbox::query::query_activity(
                &conn, from, to, config.session_gap_minutes, config.first_commit_minutes,
            )?;
            let tickets = blackbox::insights::aggregate_time_per_ticket(&repos, &config.ticket_patterns);
            match format {
                OutputFormat::Json => {
                    let json: Vec<serde_json::Value> = tickets.iter().map(|t| {
                        serde_json::json!({
                            "ticket_id": t.ticket_id,
                            "branches": t.branches,
                            "repos": t.repos,
                            "commits": t.commits,
                            "estimated_minutes": t.estimated_minutes,
                        })
                    }).collect();
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
                OutputFormat::Csv => {
                    let mut wtr = csv::Writer::from_writer(vec![]);
                    wtr.write_record(["ticket_id", "branches", "repos", "commits", "estimated_minutes"]).unwrap();
                    for t in &tickets {
                        wtr.write_record([
                            &t.ticket_id,
                            &t.branches.join(";"),
                            &t.repos.join(";"),
                            &t.commits.to_string(),
                            &t.estimated_minutes.to_string(),
                        ]).unwrap();
                    }
                    let data = String::from_utf8(wtr.into_inner().unwrap()).unwrap();
                    print!("{}", data);
                }
                _ => {
                    println!("{}", blackbox::output::render_tickets(&tickets));
                }
            }
        }
        Commands::Churn { range, threshold, format } => {
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)
                .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
            let (from, to) = range.to_range();
            let entries = blackbox::db::query_churn(
                &conn,
                &from.to_rfc3339(),
                &to.to_rfc3339(),
                threshold,
            )?;
            match format {
                OutputFormat::Json => {
                    let json: Vec<serde_json::Value> = entries.iter().map(|e| {
                        serde_json::json!({
                            "file_path": e.file_path,
                            "change_count": e.change_count,
                            "repo_path": e.repo_path,
                        })
                    }).collect();
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
                OutputFormat::Csv => {
                    let mut wtr = csv::Writer::from_writer(vec![]);
                    wtr.write_record(["file_path", "change_count", "repo_path"]).unwrap();
                    for e in &entries {
                        wtr.write_record([
                            &e.file_path,
                            &e.change_count.to_string(),
                            &e.repo_path,
                        ]).unwrap();
                    }
                    let data = String::from_utf8(wtr.into_inner().unwrap()).unwrap();
                    print!("{}", data);
                }
                _ => {
                    println!("{}", blackbox::output::render_churn(&entries));
                }
            }
        }
        Commands::Focus { range, format } => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)
                .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
            let (from, to) = range.to_range();
            let repos = blackbox::query::query_activity(
                &conn, from, to, config.session_gap_minutes, config.first_commit_minutes,
            )?;
            let sessions = blackbox::insights::deep_work_sessions(&repos, config.deep_work_threshold_minutes as i64);
            let total_minutes: i64 = repos.iter().map(|r| r.estimated_time.num_minutes()).sum();
            match format {
                OutputFormat::Json => {
                    let json: Vec<serde_json::Value> = sessions.iter().map(|s| {
                        serde_json::json!({
                            "repo_name": s.repo_name,
                            "branch": s.branch,
                            "duration_minutes": s.duration_minutes,
                            "commit_count": s.commit_count,
                        })
                    }).collect();
                    let wrapper = serde_json::json!({
                        "sessions": json,
                        "total_deep_work_minutes": sessions.iter().map(|s| s.duration_minutes).sum::<i64>(),
                        "total_estimated_minutes": total_minutes,
                        "focus_pct": if total_minutes > 0 {
                            (sessions.iter().map(|s| s.duration_minutes).sum::<i64>() as f64 / total_minutes as f64 * 100.0).round()
                        } else { 0.0 },
                    });
                    println!("{}", serde_json::to_string_pretty(&wrapper).unwrap());
                }
                _ => {
                    println!("{}", blackbox::output::render_focus(&sessions, total_minutes));
                }
            }
        }
        Commands::Trends { format } => {
            let config = blackbox::config::load_config()?;
            let data_dir = blackbox::config::data_dir()?;
            let db_path = data_dir.join("blackbox.db");
            let conn = blackbox::db::open_db(&db_path)
                .with_context(|| format!("Failed to open DB at {}", db_path.display()))?;
            let now = chrono::Utc::now();
            let from = now - chrono::Duration::days(31);
            let repos = blackbox::query::query_activity(
                &conn, from, now, config.session_gap_minutes, config.first_commit_minutes,
            )?;
            let daily = blackbox::insights::daily_estimated_minutes(&repos);
            match format {
                OutputFormat::Json => {
                    let today = chrono::Local::now().date_naive();
                    let mut entries: Vec<serde_json::Value> = Vec::new();
                    for i in (0..30).rev() {
                        let date = today - chrono::Duration::days(i);
                        let mins = daily.get(&date).copied().unwrap_or(0);
                        entries.push(serde_json::json!({
                            "date": date.to_string(),
                            "estimated_minutes": mins,
                        }));
                    }
                    println!("{}", serde_json::to_string_pretty(&entries).unwrap());
                }
                _ => {
                    println!("{}", blackbox::output::render_trends(&daily));
                }
            }
        }
        Commands::Retro { sprint, format } => {
            // Parse sprint: strip 'w' suffix, multiply by 7
            let weeks: i64 = sprint.trim_end_matches('w').parse()
                .map_err(|_| anyhow::anyhow!("Invalid sprint format '{}'. Use 1w, 2w, 3w, or 4w.", sprint))?;
            let now = chrono::Utc::now();
            let from = now - chrono::Duration::weeks(weeks);

            let (config, summary) = build_summary(&format!("Sprint ({})", sprint), from, now, false)?;

            let retro = blackbox::insights::retro_summary(
                &summary,
                config.work_hours_start,
                config.work_hours_end,
                config.deep_work_threshold_minutes as i64,
            );

            match format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&retro).unwrap());
                }
                _ => {
                    println!("{}", blackbox::output::render_retro(&retro, &sprint));
                }
            }
        }
        Commands::Metrics { range, format } => {
            let (from, to) = range.to_range();
            let period_days = (to - from).num_days();
            let (_config, summary) = build_summary("Metrics", from, to, true)?;

            let metrics = blackbox::insights::dora_lite_metrics(&summary, period_days, from.date_naive(), to.date_naive());
            match format {
                OutputFormat::Json => {
                    let json = serde_json::json!({
                        "commits_per_day": metrics.commits_per_day,
                        "prs_merged_per_week": metrics.prs_merged_per_week,
                        "velocity_trend": metrics.velocity_trend,
                        "period_days": period_days,
                    });
                    println!("{}", serde_json::to_string_pretty(&json).unwrap());
                }
                _ => {
                    println!("{}", blackbox::output::render_metrics(&metrics));
                }
            }
=======
            run_query(
                "This Month",
                blackbox::query::month_range,
                format,
                summarize,
            )?;
>>>>>>> 0e38861 (fix: resolve all clippy warnings and fmt issues)
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
            let (label, range_fn): (&str, fn() -> (DateTime<Utc>, DateTime<Utc>)) = if week {
                ("This Week", blackbox::query::week_range)
            } else {
                ("Today", blackbox::query::today_range)
            };
            let (from, to) = range_fn();
            let (config, summary) = build_summary(label, from, to, false)?;
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
