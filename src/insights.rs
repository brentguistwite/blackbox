use crate::output::OutputFormat;

pub fn run_insights(window: Option<&str>, format: OutputFormat) -> anyhow::Result<()> {
    let config = crate::config::load_config()?;
    let data_dir = crate::config::data_dir()?;
    let db_path = data_dir.join("blackbox.db");
    let conn = crate::db::open_db(&db_path)?;

    // CLI arg > config > "week"
    let effective_window = window
        .or(config.insights_window.as_deref())
        .unwrap_or("week");

    let (from, to) = match effective_window {
        "month" => crate::query::month_range(),
        _ => crate::query::week_range(),
    };
    let window_label = match effective_window {
        "month" => "This Month",
        _ => "This Week",
    };

    let mut repos = crate::query::query_activity(
        &conn,
        from,
        to,
        config.session_gap_minutes,
        config.first_commit_minutes,
    )?;
    crate::enrichment::enrich_with_prs(&mut repos);

    let data = crate::query::aggregate_insights_data(&repos, window_label);

    match format {
        OutputFormat::Json => {
            println!("{}", crate::output::render_insights_json(&data));
        }
        OutputFormat::Pretty => {
            if data.total_commits == 0 && data.total_repos == 0 {
                println!(
                    "No activity recorded for {}. Nothing to analyze.",
                    window_label
                );
                return Ok(());
            }
            eprintln!(
                "Analyzing {} commits across {} repos...",
                data.total_commits, data.total_repos
            );
            let llm_config = crate::llm::build_llm_config(&config)?;
            let prompt = crate::llm::build_insights_prompt(&data);
            let max_tokens = config.insights_max_tokens.unwrap_or(1024);
            crate::llm::generate_insights(&llm_config, &prompt, max_tokens)?;
        }
        OutputFormat::Csv => anyhow::bail!("--format csv not supported for insights"),
    }
    Ok(())
}
