use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use rusty_venture_actions::repo::{run_repo_analysis, FinalReport, MaturityScore, RepoAnalysisRequest};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "rusty-venture",
    about = "An LLM-powered bot that analyzes git repositories and executes scripted workflows",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze a git repository for language, dependencies, security issues, and maturity.
    Analyze(AnalyzeArgs),

    /// Show past scan history from the local database.
    History(HistoryArgs),
}

#[derive(Parser)]
struct AnalyzeArgs {
    /// Git repository URL to analyze (e.g. https://github.com/user/repo)
    repo_url: String,

    /// Branch to checkout (defaults to the repository's default branch)
    #[arg(long, short = 'b')]
    branch: Option<String>,

    /// Output format
    #[arg(long, short = 'o', default_value = "text")]
    output: OutputFormat,

    /// Anthropic API key for Claude. Can also be set via ANTHROPIC_API_KEY env var.
    #[arg(long, env = "ANTHROPIC_API_KEY")]
    api_key: String,

    /// SQLite database URL for persisting results.
    /// Set to "" to skip persistence. Defaults to sqlite://rusty-venture.db.
    #[arg(long, env = "DATABASE_URL", default_value = "sqlite://rusty-venture.db")]
    database_url: String,
}

#[derive(Parser)]
struct HistoryArgs {
    /// Filter by repository URL (shows all repos if omitted)
    #[arg(long)]
    repo: Option<String>,

    /// Number of results to show
    #[arg(long, short = 'n', default_value = "20")]
    limit: i64,

    /// SQLite database URL
    #[arg(long, env = "DATABASE_URL", default_value = "sqlite://rusty-venture.db")]
    database_url: String,
}

#[derive(ValueEnum, Clone, Debug)]
enum OutputFormat {
    /// Human-readable terminal output
    Text,
    /// Machine-readable JSON
    Json,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Analyze(args) => run_analyze(args).await,
        Commands::History(args) => run_history(args).await,
    }
}

async fn run_analyze(args: AnalyzeArgs) -> Result<()> {
    eprintln!("Analyzing repository: {}", args.repo_url);

    let result = run_repo_analysis(RepoAnalysisRequest {
        repo_url: args.repo_url,
        branch: args.branch,
        claude_api_key: args.api_key,
        docker_socket: None,
    })
    .await?;

    // Persist to database if DATABASE_URL is non-empty
    if !args.database_url.is_empty() {
        match rusty_venture_store::open_pool(Some(&args.database_url)).await {
            Ok(pool) => {
                // Retrieve audit report from the result context is not available here;
                // use an empty audit report for violation persistence (violations
                // are still captured in the raw FinalReport JSON in the scans table).
                let empty_audit = rusty_venture_actions::repo::AuditReport::default();
                if let Err(e) = rusty_venture_store::insert_scan(&pool, &result, &result.maturity, &empty_audit).await {
                    eprintln!("Warning: failed to persist scan to database: {e}");
                }
            }
            Err(e) => eprintln!("Warning: could not open database: {e}"),
        }
    }

    match args.output {
        OutputFormat::Text => print_report_text(&result.report, &result.maturity, result.duration_ms),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
    }

    Ok(())
}

async fn run_history(args: HistoryArgs) -> Result<()> {
    let pool = rusty_venture_store::open_pool(Some(&args.database_url)).await?;

    let scans = match &args.repo {
        Some(url) => rusty_venture_store::list_scans_for_repo(&pool, url, args.limit).await?,
        None => rusty_venture_store::list_scans(&pool, args.limit).await?,
    };

    if scans.is_empty() {
        println!("No scans found. Run `rusty-venture analyze <repo>` to get started.");
        return Ok(());
    }

    println!();
    println!("{:<44} {:<12} {:<10} {:<10} {:<12}", "SCAN ID", "DATE", "RISK", "MATURITY", "GRADE");
    println!("{:<44} {:<12} {:<10} {:<10} {:<12}", "REPO", "", "", "", "");
    println!("{}", "─".repeat(90));

    for scan in &scans {
        let date = &scan.scanned_at[..10]; // YYYY-MM-DD
        println!(
            "{:<44} {:<12} {:<10} {:<10} {:<12}",
            truncate(&scan.id, 44),
            date,
            format!("{}/100", scan.risk_score),
            format!("{}/100", scan.composite_maturity),
            scan.maturity_grade,
        );
        println!("  {}", truncate(&scan.repo_url, 86));
        println!();
    }

    Ok(())
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

fn print_report_text(report: &FinalReport, maturity: &MaturityScore, duration_ms: u64) {
    let risk_label = match report.risk_score {
        0..=20 => "LOW",
        21..=50 => "MEDIUM",
        51..=80 => "HIGH",
        _ => "CRITICAL",
    };

    println!();
    println!("==========================================================");
    println!(" RUSTY-VENTURE REPOSITORY ANALYSIS REPORT");
    println!("==========================================================");
    println!(" Risk Score:     {}/100 [{}]", report.risk_score, risk_label);
    println!(
        " Maturity Score: {}/100 [{}]",
        maturity.composite,
        maturity.grade.label()
    );
    println!(" Analysis time:  {}ms", duration_ms);
    println!("==========================================================");
    println!();
    println!("SUMMARY");
    println!("-------");
    println!("{}", report.summary);

    // Maturity dimension breakdown
    println!();
    println!("MATURITY BREAKDOWN");
    println!("------------------");
    for dim in &maturity.dimensions {
        let bar = score_bar(dim.score);
        println!(
            "  {:22} {:3}/100  {}",
            dim.dimension.label(),
            dim.score,
            bar
        );
    }

    // Failed signals as actionable recommendations
    let failed: Vec<_> = maturity
        .dimensions
        .iter()
        .flat_map(|d| d.signals.iter())
        .filter(|s| !s.passed)
        .collect();

    if !failed.is_empty() {
        println!();
        println!("MATURITY IMPROVEMENT ACTIONS");
        println!("----------------------------");
        for s in &failed {
            if let Some(detail) = &s.detail {
                println!("  • [{}] {}", s.name, detail);
            } else {
                println!("  • [{}] {}", s.name, s.description);
            }
        }
    }

    print_section("LANGUAGE INSIGHTS", &report.language_insights);
    print_section("DEPENDENCY RECOMMENDATIONS", &report.dependency_recommendations);
    print_section("DOCKERFILE FINDINGS", &report.dockerfile_findings);
    print_section("SECURITY VIOLATIONS", &report.security_violations);
    print_section("GENERAL RECOMMENDATIONS", &report.general_recommendations);

    println!("==========================================================");
}

fn score_bar(score: u8) -> String {
    let filled = (score as usize / 10).min(10);
    let empty = 10 - filled;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

fn print_section(title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    println!();
    println!("{title}");
    println!("{}", "-".repeat(title.len()));
    for item in items {
        println!("  • {item}");
    }
}
