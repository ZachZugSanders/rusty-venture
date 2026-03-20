use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use rusty_venture_actions::repo::{
    detect_language::{DetectedLanguages, Language},
    governance::GovernanceReport,
    run_repo_analysis, FinalReport, MaturityScore, RepoAnalysisRequest,
    CTX_DETECTED_LANGUAGES, CTX_REPO_URL,
};
use rusty_venture_core::context::ExecutionContext;
use rusty_venture_core::action::Action;
use rusty_venture_improve::{ContainerizeAction, GenerateGovernanceFilesAction, GovernanceFile};
use rusty_venture_llm::ClaudeConnector;
use tracing_subscriber::EnvFilter;
use std::path::Path;
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

    /// Generate missing governance files for a local repository (dry-run by default).
    ///
    /// Checks for LICENSE, SECURITY.md, CONTRIBUTING.md, CHANGELOG.md,
    /// .github/dependabot.yml, and language-specific lint/safety configs.
    /// With --dry-run (default), only lists what would be created.
    /// Without --dry-run, writes the files to --output-dir.
    Improve(ImproveArgs),
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

    /// Skip Docker container spin-up and analyse using only local filesystem
    /// reads and the LLM. Requires `git` to be available on PATH.
    /// Dependency scanning and file-audit checks are skipped in this mode.
    #[arg(long, short = 'n')]
    no_container: bool,
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

/// Arguments for the `improve` subcommand.
#[derive(Parser)]
struct ImproveArgs {
    /// Path to the local repository to improve (defaults to current directory).
    #[arg(default_value = ".")]
    repo_path: String,

    /// List which files would be created without writing them.
    /// Defaults to true; pass --dry-run=false to actually write files.
    #[arg(long, default_value = "true")]
    dry_run: bool,

    /// Directory to write generated files into (ignored when --dry-run is set).
    /// Defaults to the repo path itself.
    #[arg(long)]
    output_dir: Option<String>,

    /// Also generate a Dockerfile (and docker-compose.yml if applicable) using
    /// LLM + an iterative docker-build validation loop.
    #[arg(long)]
    containerize: bool,

    /// Remote git URL of the repository (required when --containerize is set).
    #[arg(long)]
    repo_url: Option<String>,

    /// Anthropic API key (required when --containerize is set).
    /// Falls back to the ANTHROPIC_API_KEY environment variable.
    #[arg(long, env = "ANTHROPIC_API_KEY")]
    api_key: Option<String>,
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
    dotenvy::dotenv().ok(); // load .env if present — no-op if file is missing

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
        Commands::Improve(args) => run_improve(args).await,
    }
}

async fn run_improve(args: ImproveArgs) -> Result<()> {
    let repo_path = Path::new(&args.repo_path).canonicalize()
        .unwrap_or_else(|_| Path::new(&args.repo_path).to_path_buf());

    eprintln!("Scanning repository: {}", repo_path.display());

    // ── Phase 0: Governance files ──────────────────────────────────────────
    let primary_lang = detect_language_local(&repo_path);
    let languages = DetectedLanguages {
        secondary: vec![],
        scores: vec![(primary_lang.clone(), 10)],
        primary: primary_lang,
    };
    let gov = detect_governance_local(&repo_path);
    let ctx = ExecutionContext::new("improve-governance");
    ctx.insert(CTX_DETECTED_LANGUAGES, languages.clone()).await;
    ctx.insert(rusty_venture_actions::repo::CTX_GOVERNANCE_REPORT, gov).await;

    let gov_files: Vec<GovernanceFile> = GenerateGovernanceFilesAction
        .execute(&ctx, ())
        .await
        .map_err(|e| anyhow::anyhow!("Governance generation failed: {e}"))?;

    let out_dir = args.output_dir.as_deref().unwrap_or(&args.repo_path);
    let out_path = Path::new(out_dir);

    if gov_files.is_empty() {
        eprintln!("✓  All governance files are already present.");
    } else if args.dry_run {
        println!();
        println!("Governance files that would be created ({} total):", gov_files.len());
        for f in &gov_files {
            println!("  + {}", f.path);
        }
    } else {
        for f in &gov_files {
            let dest = out_path.join(&f.path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&dest, &f.content)?;
            println!("  created  {}", dest.display());
        }
        println!();
        println!("✓  {} governance file(s) created.", gov_files.len());
    }

    // ── Phase 1: Containerization (opt-in via --containerize) ─────────────
    if args.containerize {
        let repo_url = args.repo_url.as_deref().ok_or_else(|| {
            anyhow::anyhow!("--repo-url <url> is required when --containerize is set")
        })?;
        let api_key = args
            .api_key
            .clone()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--api-key or ANTHROPIC_API_KEY env var is required when --containerize is set"
                )
            })?;

        eprintln!("Generating Dockerfile via LLM for {repo_url} ...");

        let connector = std::sync::Arc::new(ClaudeConnector::new(&api_key));
        let container_ctx = ExecutionContext::new("improve-containerize");
        container_ctx.insert(CTX_REPO_URL, repo_url.to_string()).await;
        container_ctx.insert(CTX_DETECTED_LANGUAGES, languages).await;

        let c_result = ContainerizeAction::new(connector)
            .execute(&container_ctx, ())
            .await
            .map_err(|e| anyhow::anyhow!("Containerization failed: {e}"))?;

        let validated_label = if c_result.build_validated { "validated" } else { "not validated" };

        if args.dry_run {
            println!(
                "  + {} (LLM-generated, {} fix iteration(s), build {})",
                c_result.dockerfile_path, c_result.fix_iterations, validated_label
            );
            if c_result.compose_content.is_some() {
                println!("  + docker-compose.yml");
            }
        } else {
            let docker_dest = out_path.join(&c_result.dockerfile_path);
            std::fs::write(&docker_dest, &c_result.dockerfile_content)?;
            println!("  created  {}", docker_dest.display());
            if let Some(compose) = &c_result.compose_content {
                let compose_dest = out_path.join("docker-compose.yml");
                std::fs::write(&compose_dest, compose)?;
                println!("  created  {}", compose_dest.display());
            }
            println!();
            println!("✓  Dockerfile created ({}).", validated_label);
        }
    }

    if args.dry_run {
        println!();
        println!("Re-run with --dry-run=false to write the listed files.");
    }

    Ok(())
}

/// Detect primary language from well-known marker files in the repo directory.
fn detect_language_local(repo_path: &Path) -> Language {
    let markers: &[(&str, Language)] = &[
        ("Cargo.toml", Language::Rust),
        ("package.json", Language::Node),
        ("pyproject.toml", Language::Python),
        ("requirements.txt", Language::Python),
        ("go.mod", Language::Go),
        ("pom.xml", Language::Java),
        ("build.gradle", Language::Java),
        ("Gemfile", Language::Ruby),
        ("composer.json", Language::PHP),
    ];

    for (file, lang) in markers {
        if repo_path.join(file).exists() {
            return lang.clone();
        }
    }
    Language::Unknown
}

/// Check which governance files are already present in the repo directory.
fn detect_governance_local(repo_path: &Path) -> GovernanceReport {
    let has = |name: &str| repo_path.join(name).exists();
    let has_any = |names: &[&str]| names.iter().any(|n| has(n));

    GovernanceReport {
        has_license: has_any(&["LICENSE", "LICENSE.md", "LICENSE.txt", "LICENCE"]),
        has_readme: has_any(&["README.md", "README.txt", "README.rst", "README"]),
        has_changelog: has_any(&["CHANGELOG.md", "CHANGELOG.txt", "HISTORY.md"]),
        has_contributing: has_any(&["CONTRIBUTING.md", "CONTRIBUTING.txt"]),
        has_security_policy: has_any(&[
            "SECURITY.md",
            ".github/SECURITY.md",
        ]),
        has_dependabot: has(".github/dependabot.yml"),
        has_lint_config: has_any(&[
            "clippy.toml",
            ".clippy.toml",
            "eslint.config.js",
            ".eslintrc.js",
            ".eslintrc.json",
            ".eslintrc.yml",
            "ruff.toml",
            ".ruff.toml",
            ".golangci.yml",
            ".golangci.yaml",
        ]),
        has_safety_config: has_any(&["deny.toml", "mypy.ini", ".mypy.ini"]),
        ..Default::default()
    }
}

async fn run_analyze(args: AnalyzeArgs) -> Result<()> {
    eprintln!("Analyzing repository: {}", args.repo_url);

    let result = run_repo_analysis(RepoAnalysisRequest {
        repo_url: args.repo_url,
        branch: args.branch,
        claude_api_key: args.api_key,
        docker_socket: None,
        skip_container: args.no_container,
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
