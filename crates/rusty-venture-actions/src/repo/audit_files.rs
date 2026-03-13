use std::sync::Arc;

use async_trait::async_trait;
use bollard::Docker;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::container::{exec_in_container, ExecCommand, CTX_CONTAINER_ID, CTX_DOCKER_CLIENT};
use super::clone::CTX_REPO_LOCAL_PATH;
use super::detect_language::{CTX_DETECTED_LANGUAGES, DetectedLanguages, Language};

pub const CTX_AUDIT_REPORT: &str = "repo.audit_report";

/// A file that should not have been committed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditViolation {
    pub file: String,
    pub matched_pattern: String,
    pub severity: ViolationSeverity,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ViolationSeverity {
    Info,
    Warning,
    Critical,
}

impl std::fmt::Display for ViolationSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ViolationSeverity::Info => "Info",
            ViolationSeverity::Warning => "Warning",
            ViolationSeverity::Critical => "Critical",
        };
        write!(f, "{s}")
    }
}

/// The result of the file audit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub violations: Vec<AuditViolation>,
    pub total_tracked_files: usize,
    pub critical_count: usize,
    pub warning_count: usize,
}

/// A deny-list entry: a glob-like pattern and its associated severity/message.
struct DenyEntry {
    pattern: &'static str,
    severity: ViolationSeverity,
    recommendation: &'static str,
}

impl DenyEntry {
    const fn new(pattern: &'static str, severity: ViolationSeverity, recommendation: &'static str) -> Self {
        Self { pattern, severity, recommendation }
    }

    fn matches(&self, file: &str) -> bool {
        let filename = file.split('/').next_back().unwrap_or(file);
        let pattern = self.pattern;

        if pattern.ends_with('/') {
            // Directory pattern: match any file under this directory
            let dir = pattern.trim_end_matches('/');
            return file.contains(&format!("{dir}/")) || file == dir;
        }

        if pattern.starts_with("*.") {
            // Extension pattern
            return filename.ends_with(&pattern[1..]);
        }

        if pattern.contains('*') {
            // Simple glob: prefix match before *
            let parts: Vec<&str> = pattern.splitn(2, '*').collect();
            let prefix = parts[0];
            let suffix = parts.get(1).copied().unwrap_or("");
            return filename.starts_with(prefix) && filename.ends_with(suffix);
        }

        // Exact filename match
        filename == pattern || file == pattern
    }
}

// Universal deny list (applies to all languages)
static DENY_UNIVERSAL: &[DenyEntry] = &[
    DenyEntry::new(".env", ViolationSeverity::Critical, "Remove .env and add it to .gitignore. Use .env.example for templates."),
    DenyEntry::new(".env.local", ViolationSeverity::Critical, "Remove .env.local — it may contain local secrets."),
    DenyEntry::new(".env.production", ViolationSeverity::Critical, "Remove .env.production — NEVER commit production credentials."),
    DenyEntry::new(".env.staging", ViolationSeverity::Critical, "Remove .env.staging — NEVER commit environment credentials."),
    DenyEntry::new("*.pem", ViolationSeverity::Critical, "Remove certificate file and rotate any exposed keys immediately."),
    DenyEntry::new("*.key", ViolationSeverity::Critical, "Remove private key file and rotate the key immediately."),
    DenyEntry::new("*.p12", ViolationSeverity::Critical, "Remove PKCS#12 certificate bundle."),
    DenyEntry::new("*.pfx", ViolationSeverity::Critical, "Remove PFX certificate file."),
    DenyEntry::new("id_rsa", ViolationSeverity::Critical, "Remove SSH private key and rotate it immediately."),
    DenyEntry::new("id_ed25519", ViolationSeverity::Critical, "Remove SSH private key and rotate it immediately."),
    DenyEntry::new("id_dsa", ViolationSeverity::Critical, "Remove SSH private key and rotate it immediately."),
    DenyEntry::new("*.ppk", ViolationSeverity::Critical, "Remove PuTTY private key file."),
    DenyEntry::new("credentials.json", ViolationSeverity::Critical, "Remove credentials file — may contain API keys or OAuth tokens."),
    DenyEntry::new("secrets.yaml", ViolationSeverity::Critical, "Remove secrets file — add to .gitignore."),
    DenyEntry::new("secrets.yml", ViolationSeverity::Critical, "Remove secrets file — add to .gitignore."),
    DenyEntry::new(".DS_Store", ViolationSeverity::Warning, "Remove macOS metadata file and add .DS_Store to .gitignore."),
    DenyEntry::new("Thumbs.db", ViolationSeverity::Info, "Remove Windows thumbnail cache and add to .gitignore."),
    DenyEntry::new("*.log", ViolationSeverity::Warning, "Remove log files and add *.log to .gitignore."),
    DenyEntry::new("*.tmp", ViolationSeverity::Info, "Remove temp files and add *.tmp to .gitignore."),
    DenyEntry::new("*.bak", ViolationSeverity::Info, "Remove backup files and add *.bak to .gitignore."),
    DenyEntry::new("*.swp", ViolationSeverity::Info, "Remove Vim swap file and add *.swp to .gitignore."),
    DenyEntry::new("*.orig", ViolationSeverity::Info, "Remove .orig conflict resolution file."),
];

static DENY_RUST: &[DenyEntry] = &[
    DenyEntry::new("target/", ViolationSeverity::Warning, "Remove the target/ build directory — it should be in .gitignore."),
    DenyEntry::new("*.rs.bk", ViolationSeverity::Info, "Remove rustfmt backup files and add *.rs.bk to .gitignore."),
    DenyEntry::new("*.pdb", ViolationSeverity::Info, "Remove Windows debug symbol files."),
];

static DENY_NODE: &[DenyEntry] = &[
    DenyEntry::new("node_modules/", ViolationSeverity::Critical, "Remove node_modules/ — it must never be committed. Add to .gitignore and use npm/yarn install."),
    DenyEntry::new("npm-debug.log*", ViolationSeverity::Warning, "Remove npm debug logs."),
    DenyEntry::new("yarn-error.log*", ViolationSeverity::Warning, "Remove yarn error logs."),
    DenyEntry::new(".next/", ViolationSeverity::Warning, "Remove .next/ build output — add to .gitignore."),
    DenyEntry::new("dist/", ViolationSeverity::Warning, "Remove dist/ build output — add to .gitignore."),
    DenyEntry::new("coverage/", ViolationSeverity::Info, "Remove test coverage output — add to .gitignore."),
    DenyEntry::new(".nyc_output/", ViolationSeverity::Info, "Remove NYC coverage output — add to .gitignore."),
    DenyEntry::new("*.tsbuildinfo", ViolationSeverity::Info, "Remove TypeScript incremental build info — add to .gitignore."),
];

static DENY_PYTHON: &[DenyEntry] = &[
    DenyEntry::new("__pycache__/", ViolationSeverity::Warning, "Remove __pycache__/ and add to .gitignore."),
    DenyEntry::new("*.pyc", ViolationSeverity::Warning, "Remove compiled Python files and add *.pyc to .gitignore."),
    DenyEntry::new("*.pyo", ViolationSeverity::Warning, "Remove optimized Python files."),
    DenyEntry::new(".venv/", ViolationSeverity::Critical, "Remove .venv/ virtual environment — add to .gitignore."),
    DenyEntry::new("venv/", ViolationSeverity::Critical, "Remove venv/ virtual environment — add to .gitignore."),
    DenyEntry::new(".pytest_cache/", ViolationSeverity::Info, "Remove pytest cache — add to .gitignore."),
    DenyEntry::new(".mypy_cache/", ViolationSeverity::Info, "Remove mypy cache — add to .gitignore."),
    DenyEntry::new(".ruff_cache/", ViolationSeverity::Info, "Remove ruff cache — add to .gitignore."),
    DenyEntry::new("*.egg-info/", ViolationSeverity::Warning, "Remove egg-info directory — add to .gitignore."),
    DenyEntry::new("dist/", ViolationSeverity::Warning, "Remove Python dist/ build output."),
];

static DENY_GO: &[DenyEntry] = &[
    DenyEntry::new("vendor/", ViolationSeverity::Warning, "Consider whether vendor/ should be committed. Use Go modules instead."),
    DenyEntry::new("bin/", ViolationSeverity::Warning, "Remove compiled Go binaries from the repository."),
];

static DENY_JAVA: &[DenyEntry] = &[
    DenyEntry::new("target/", ViolationSeverity::Warning, "Remove Maven target/ directory — add to .gitignore."),
    DenyEntry::new("*.class", ViolationSeverity::Warning, "Remove compiled .class files — add to .gitignore."),
    DenyEntry::new("*.jar", ViolationSeverity::Warning, "Remove JAR files from source control — use a package manager."),
    DenyEntry::new("*.war", ViolationSeverity::Warning, "Remove WAR files from source control."),
    DenyEntry::new("build/", ViolationSeverity::Warning, "Remove Gradle build/ output — add to .gitignore."),
    DenyEntry::new(".gradle/", ViolationSeverity::Warning, "Remove .gradle/ cache — add to .gitignore."),
];

static DENY_RUBY: &[DenyEntry] = &[
    DenyEntry::new(".bundle/", ViolationSeverity::Warning, "Remove .bundle/ config — add to .gitignore."),
    DenyEntry::new("*.gem", ViolationSeverity::Warning, "Remove built gem files from source control."),
    DenyEntry::new(".byebug_history", ViolationSeverity::Info, "Remove Byebug history — add to .gitignore."),
];

static DENY_PHP: &[DenyEntry] = &[
    DenyEntry::new("vendor/", ViolationSeverity::Critical, "Remove vendor/ — it should be installed via Composer, not committed."),
    DenyEntry::new("storage/", ViolationSeverity::Warning, "Laravel storage/ should generally not be committed."),
    DenyEntry::new("bootstrap/cache/", ViolationSeverity::Warning, "Remove bootstrap/cache/ — add to .gitignore."),
];

static DENY_CSHARP: &[DenyEntry] = &[
    DenyEntry::new("bin/", ViolationSeverity::Warning, "Remove bin/ build output — add to .gitignore."),
    DenyEntry::new("obj/", ViolationSeverity::Warning, "Remove obj/ build output — add to .gitignore."),
    DenyEntry::new(".vs/", ViolationSeverity::Info, "Remove .vs/ Visual Studio cache — add to .gitignore."),
    DenyEntry::new("*.nupkg", ViolationSeverity::Warning, "Remove NuGet packages — use a package feed instead."),
    DenyEntry::new("TestResults/", ViolationSeverity::Info, "Remove test results directory — add to .gitignore."),
];

/// Checks tracked git files against per-language deny lists.
#[derive(Default)]
pub struct AuditCommittedFilesAction;

#[async_trait]
impl Action for AuditCommittedFilesAction {
    type Input = ();
    type Output = AuditReport;

    fn name(&self) -> &str {
        "audit-committed-files"
    }

    async fn execute(&self, ctx: &ExecutionContext, _input: ()) -> Result<AuditReport, CoreError> {
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;
        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;
        let repo_path: String = ctx.require::<String>(CTX_REPO_LOCAL_PATH).await?;
        let detected: DetectedLanguages = ctx.require::<DetectedLanguages>(CTX_DETECTED_LANGUAGES).await?;

        // Get all tracked files via git ls-files
        let ls_result = exec_in_container(
            &docker,
            &container_id,
            ExecCommand::new(["git", "ls-files"]).working_dir(&repo_path),
        ).await?;

        // Also check untracked but present files (shouldn't be committed but are there)
        let find_result = exec_in_container(
            &docker,
            &container_id,
            ExecCommand::new([
                "find", &repo_path,
                "-not", "-path", "*/.git/*",
                "-type", "f",
            ]),
        ).await?;

        let tracked_files: Vec<String> = ls_result.stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(String::from)
            .collect();

        let all_files: Vec<String> = find_result.stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.strip_prefix(&format!("{repo_path}/")).unwrap_or(l).to_string())
            .collect();

        // Use the union of tracked + all present files for maximum coverage
        let mut files_to_check: Vec<String> = tracked_files.clone();
        for f in &all_files {
            if !files_to_check.contains(f) {
                files_to_check.push(f.clone());
            }
        }

        // Build the applicable deny lists
        let language_deny: &[DenyEntry] = match &detected.primary {
            Language::Rust => DENY_RUST,
            Language::Node => DENY_NODE,
            Language::Python => DENY_PYTHON,
            Language::Go => DENY_GO,
            Language::Java => DENY_JAVA,
            Language::Ruby => DENY_RUBY,
            Language::PHP => DENY_PHP,
            Language::CSharp => DENY_CSHARP,
            _ => &[],
        };

        let all_deny: Vec<&DenyEntry> = DENY_UNIVERSAL.iter().chain(language_deny.iter()).collect();

        let mut violations = vec![];
        let mut seen = std::collections::HashSet::new();

        for file in &files_to_check {
            for entry in &all_deny {
                if entry.matches(file) {
                    let key = format!("{}:{}", file, entry.pattern);
                    if seen.insert(key) {
                        violations.push(AuditViolation {
                            file: file.clone(),
                            matched_pattern: entry.pattern.to_string(),
                            severity: entry.severity.clone(),
                            recommendation: entry.recommendation.to_string(),
                        });
                    }
                }
            }
        }

        // Sort by severity (critical first)
        violations.sort_by(|a, b| b.severity.cmp(&a.severity));

        let critical_count = violations.iter().filter(|v| v.severity == ViolationSeverity::Critical).count();
        let warning_count = violations.iter().filter(|v| v.severity == ViolationSeverity::Warning).count();

        info!(
            total_files = files_to_check.len(),
            violations = violations.len(),
            critical = critical_count,
            "File audit complete"
        );

        let report = AuditReport {
            violations,
            total_tracked_files: tracked_files.len(),
            critical_count,
            warning_count,
        };

        ctx.insert(CTX_AUDIT_REPORT, report.clone()).await;
        Ok(report)
    }
}
