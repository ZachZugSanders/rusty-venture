/// Maturity scoring model derived from:
/// - OpenSSF Scorecard (https://github.com/ossf/scorecard)
/// - CII/OpenSSF Best Practices Badge (https://bestpractices.dev/en/criteria)
/// - ISO/IEC 25010:2023 SQuaRE — Maintainability + Security characteristics
/// - Language-specific API guidelines (Rust, Node, Python, Go)
///
/// Only signals derivable from a **static repository scan** (no external APIs,
/// no running the code) are used. Each dimension score is 0–100; the composite
/// is a weighted average rounded to the nearest integer.
use serde::{Deserialize, Serialize};

pub const CTX_MATURITY_SCORE: &str = "repo.maturity_score";

use super::{
    analyze_deps::DependencyReport,
    audit_files::AuditReport,
    detect_language::Language,
    find_dockerfiles::DockerfileReport,
    governance::GovernanceReport,
    report::FinalReport,
};

// ── Dimension weights (must sum to 1.0) ─────────────────────────────────────

const W_SECURITY: f32 = 0.25;
const W_DEPENDENCY: f32 = 0.20;
const W_BUILD_CI: f32 = 0.15;
const W_CODE_ORG: f32 = 0.15;
const W_GOVERNANCE: f32 = 0.15;
const W_TESTING: f32 = 0.10;

// ── Public API ───────────────────────────────────────────────────────────────

/// A single measurable signal within a maturity dimension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaturitySignal {
    /// Short identifier used in the database and display.
    pub name: String,
    /// Human-readable description of what this signal measures.
    pub description: String,
    /// Whether the signal passed for this repository.
    pub passed: bool,
    /// Point value of this signal (used to compute the dimension score).
    pub points: u8,
    /// Optional additional context explaining why the signal passed or failed.
    pub detail: Option<String>,
}

impl MaturitySignal {
    fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        passed: bool,
        points: u8,
        detail: impl Into<Option<String>>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            passed,
            points,
            detail: detail.into(),
        }
    }
}

/// One of the six maturity dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MaturityDimension {
    /// Credential hygiene, vulnerability exposure, security policy presence.
    Security,
    /// Lock file, manifest completeness, dependency count, pinned images.
    DependencyHealth,
    /// CI configuration, reproducible builds, safe Docker practices.
    BuildAndCi,
    /// Language-specific idioms: MSRV, lint config, appropriate structure.
    CodeOrganization,
    /// LICENSE, README, CHANGELOG, CONTRIBUTING, Code of Conduct.
    ProjectGovernance,
    /// Test files, test config, SAST/linter presence.
    TestingAndQuality,
}

impl MaturityDimension {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Security => "Security",
            Self::DependencyHealth => "Dependency Health",
            Self::BuildAndCi => "Build & CI",
            Self::CodeOrganization => "Code Organization",
            Self::ProjectGovernance => "Project Governance",
            Self::TestingAndQuality => "Testing & Quality",
        }
    }

    pub fn weight(&self) -> f32 {
        match self {
            Self::Security => W_SECURITY,
            Self::DependencyHealth => W_DEPENDENCY,
            Self::BuildAndCi => W_BUILD_CI,
            Self::CodeOrganization => W_CODE_ORG,
            Self::ProjectGovernance => W_GOVERNANCE,
            Self::TestingAndQuality => W_TESTING,
        }
    }
}

/// Scored result for one dimension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionScore {
    pub dimension: MaturityDimension,
    /// Normalised score 0–100 for this dimension.
    pub score: u8,
    /// Individual signals that were evaluated.
    pub signals: Vec<MaturitySignal>,
}

impl DimensionScore {
    fn compute(dimension: MaturityDimension, signals: Vec<MaturitySignal>) -> Self {
        let total: u32 = signals.iter().map(|s| s.points as u32).sum();
        let earned: u32 = signals.iter().filter(|s| s.passed).map(|s| s.points as u32).sum();
        let score = if total == 0 {
            0u8
        } else {
            ((earned as f32 / total as f32) * 100.0).round() as u8
        };
        Self { dimension, score, signals }
    }
}

/// Overall maturity grade mapped from the composite score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MaturityGrade {
    /// 0–20: Ad hoc processes, minimal hygiene.
    Nascent,
    /// 21–40: Some practices in place but inconsistent.
    Emerging,
    /// 41–60: Most core practices present; meaningful gaps remain.
    Developing,
    /// 61–80: Consistently applied practices; minor gaps.
    Established,
    /// 81–100: Exemplary hygiene across all dimensions.
    Exemplary,
}

impl MaturityGrade {
    pub fn from_score(score: u8) -> Self {
        match score {
            0..=20 => Self::Nascent,
            21..=40 => Self::Emerging,
            41..=60 => Self::Developing,
            61..=80 => Self::Established,
            _ => Self::Exemplary,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Nascent => "NASCENT",
            Self::Emerging => "EMERGING",
            Self::Developing => "DEVELOPING",
            Self::Established => "ESTABLISHED",
            Self::Exemplary => "EXEMPLARY",
        }
    }
}

/// The complete maturity assessment for one repository scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaturityScore {
    /// Weighted composite score 0–100.
    pub composite: u8,
    /// Maturity grade mapped from composite.
    pub grade: MaturityGrade,
    /// Per-dimension breakdown.
    pub dimensions: Vec<DimensionScore>,
}

// ── Scoring logic ────────────────────────────────────────────────────────────

/// Compute the full maturity score from all available analysis reports.
///
/// All inputs are required — pass defaults (empty structs) for steps that
/// failed gracefully; the scorer handles absence conservatively (no points).
pub fn compute_maturity(
    _report: &FinalReport,
    deps: &DependencyReport,
    dockerfiles: &DockerfileReport,
    audit: &AuditReport,
    primary_language: &Language,
    gov: &GovernanceReport,
) -> MaturityScore {
    let dimensions = vec![
        score_security(audit, gov),
        score_dependency_health(deps, dockerfiles, gov),
        score_build_and_ci(dockerfiles, gov),
        score_code_organization(primary_language, deps, gov),
        score_project_governance(gov),
        score_testing_and_quality(gov),
    ];

    let composite = {
        let weighted: f32 = dimensions
            .iter()
            .map(|d| d.score as f32 * d.dimension.weight())
            .sum();
        weighted.round() as u8
    };

    MaturityScore {
        composite,
        grade: MaturityGrade::from_score(composite),
        dimensions,
    }
}

// ── Dimension scorers ────────────────────────────────────────────────────────

/// Security (25%) — OpenSSF Scorecard static checks + CII no-credential requirement.
fn score_security(audit: &AuditReport, gov: &GovernanceReport) -> DimensionScore {
    let signals = vec![
        // No critical violations — highest weight; directly maps to OpenSSF Binary-Artifacts
        // and CII no_leaked_credentials criteria.
        MaturitySignal::new(
            "no_critical_violations",
            "No critical committed files (secrets, credentials, private keys)",
            audit.critical_count == 0,
            40,
            if audit.critical_count > 0 {
                Some(format!("{} critical violation(s) found", audit.critical_count))
            } else {
                None
            },
        ),
        // Low warning violations — proportional scoring
        MaturitySignal::new(
            "low_warning_violations",
            "Few or no warning-level committed files (log files, build artifacts)",
            audit.warning_count <= 2,
            20,
            if audit.warning_count > 2 {
                Some(format!("{} warning violation(s) found", audit.warning_count))
            } else {
                None
            },
        ),
        // Security policy — maps to OpenSSF Security-Policy check and CII vulnerability_report_process
        MaturitySignal::new(
            "security_policy",
            "SECURITY.md present describing vulnerability disclosure process",
            gov.has_security_policy,
            25,
            if !gov.has_security_policy {
                Some("Add a SECURITY.md following the OpenSSF template".to_string())
            } else {
                None
            },
        ),
        // Dependency update automation — maps to OpenSSF Dependency-Update-Tool check
        MaturitySignal::new(
            "dep_update_tool",
            "Automated dependency updates configured (Dependabot or Renovate)",
            gov.has_dependabot || gov.has_renovate,
            15,
            if !gov.has_dependabot && !gov.has_renovate {
                Some("Add .github/dependabot.yml or renovate.json".to_string())
            } else {
                None
            },
        ),
    ];
    DimensionScore::compute(MaturityDimension::Security, signals)
}

/// Dependency Health (20%) — CII external_dependencies + lock file + OpenSSF Pinned-Dependencies.
fn score_dependency_health(
    deps: &DependencyReport,
    dockerfiles: &DockerfileReport,
    gov: &GovernanceReport,
) -> DimensionScore {
    let dep_count = deps.dependencies.len();

    // Check for :latest Docker image tags from dockerfile warnings
    let has_latest_tags = dockerfiles
        .entries
        .iter()
        .any(|e| e.warnings.iter().any(|w| w.contains("latest")));

    // Lock file: use DependencyReport field + governance cross-check
    let has_lock = deps.lock_file_present || gov.has_any_lock_file;

    // Reasonable dep count: 0-30 full points, 31-60 partial, >60 none
    let dep_count_ok = dep_count <= 60;
    let dep_count_detail = if dep_count > 60 {
        Some(format!("{} direct dependencies — consider auditing for unused/redundant ones", dep_count))
    } else {
        None
    };

    let signals = vec![
        // Lock file committed — CII Silver: external_dependencies criterion
        MaturitySignal::new(
            "lock_file_present",
            "Dependency lock file committed (Cargo.lock, package-lock.json, go.sum, poetry.lock, etc.)",
            has_lock,
            40,
            if !has_lock {
                Some("Commit the lock file to ensure reproducible installs".to_string())
            } else {
                None
            },
        ),
        // Manifest file — basic hygiene, confirms project uses a proper dependency manager
        MaturitySignal::new(
            "manifest_present",
            "Dependency manifest file present (Cargo.toml, package.json, go.mod, pyproject.toml, etc.)",
            !deps.manifest_file.is_empty(),
            20,
            None,
        ),
        // Reasonable dependency count — ISO 25010 Maintainability/Modularity proxy
        MaturitySignal::new(
            "dep_count_reasonable",
            "Direct dependency count is manageable (≤60)",
            dep_count_ok,
            20,
            dep_count_detail,
        ),
        // No :latest Docker image tags — maps to OpenSSF Pinned-Dependencies check
        MaturitySignal::new(
            "no_latest_docker_tags",
            "No unpinned ':latest' image tags in Dockerfiles",
            !has_latest_tags,
            20,
            if has_latest_tags {
                Some("Pin Docker base images to specific digests or version tags".to_string())
            } else {
                None
            },
        ),
    ];
    DimensionScore::compute(MaturityDimension::DependencyHealth, signals)
}

/// Build & CI (15%) — OpenSSF Dangerous-Workflow (static) + CII CI criteria.
fn score_build_and_ci(dockerfiles: &DockerfileReport, gov: &GovernanceReport) -> DimensionScore {
    let has_dockerfile_issues = dockerfiles
        .entries
        .iter()
        .any(|e| !e.warnings.is_empty());

    let signals = vec![
        // CI configuration present — CII test_continuous_integration criterion
        MaturitySignal::new(
            "ci_config_present",
            "Continuous integration configuration present (GitHub Actions, GitLab CI, Travis, CircleCI, etc.)",
            gov.has_ci_config,
            45,
            if !gov.has_ci_config {
                Some("Add a CI pipeline (e.g. .github/workflows/ci.yml)".to_string())
            } else {
                None
            },
        ),
        // .dockerignore — prevents accidental inclusion of secrets in Docker images
        MaturitySignal::new(
            "dockerignore_present",
            ".dockerignore file present (prevents secrets/artifacts entering Docker images)",
            dockerfiles.has_dockerignore || !dockerfiles.has_root_dockerfile,
            25,
            if dockerfiles.has_root_dockerfile && !dockerfiles.has_dockerignore {
                Some("Add .dockerignore to prevent secrets and build artifacts in images".to_string())
            } else {
                None
            },
        ),
        // No dangerous Dockerfile practices (root user, privileged, :latest)
        MaturitySignal::new(
            "safe_dockerfile_practices",
            "Dockerfiles follow safe practices (no hardcoded secrets, no always-root execution)",
            !has_dockerfile_issues,
            30,
            if has_dockerfile_issues {
                Some("Review Dockerfile warnings: running as root, hardcoded secrets, or :latest tags".to_string())
            } else {
                None
            },
        ),
    ];
    DimensionScore::compute(MaturityDimension::BuildAndCi, signals)
}

/// Code Organization (15%) — ISO 25010 Maintainability + language API guidelines.
fn score_code_organization(
    primary_language: &Language,
    deps: &DependencyReport,
    gov: &GovernanceReport,
) -> DimensionScore {
    // Language-specific version constraint signal
    let has_version_constraint = match primary_language {
        Language::Rust => gov.rust_msrv_declared,
        Language::Node => gov.node_engine_declared,
        Language::Python => gov.python_requires_declared,
        Language::Go => gov.go_version_declared,
        Language::Java | Language::Ruby | Language::PHP | Language::CSharp
        | Language::Swift | Language::Kotlin => !deps.manifest_file.is_empty(),
        Language::Unknown => false,
    };

    let version_constraint_detail = if !has_version_constraint {
        let advice = match primary_language {
            Language::Rust => "Add `rust-version = \"1.XX\"` to Cargo.toml (MSRV declaration)",
            Language::Node => "Add `engines: { node: \">=XX\" }` to package.json",
            Language::Python => "Add `requires-python = \">=3.X\"` to pyproject.toml",
            Language::Go => "Ensure `go X.Y` directive is present in go.mod",
            _ => "Declare the minimum required runtime version in your manifest",
        };
        Some(advice.to_string())
    } else {
        None
    };

    let signals = vec![
        // Minimum runtime version declared — language API guidelines + CII version_semver
        MaturitySignal::new(
            "runtime_version_declared",
            "Minimum required runtime/language version declared in manifest (MSRV, engines, requires-python, go directive)",
            has_version_constraint,
            35,
            version_constraint_detail,
        ),
        // Linter configured — ISO 25010 Analysability; CII static_analysis criterion
        MaturitySignal::new(
            "linter_configured",
            "Linter or static analysis tool configured (clippy.toml, .eslintrc, ruff.toml, golangci.yml, etc.)",
            gov.has_lint_config,
            35,
            if !gov.has_lint_config {
                Some("Add linter configuration appropriate for your language".to_string())
            } else {
                None
            },
        ),
        // Pre-commit hooks — CII coding_standards_enforced (Silver)
        MaturitySignal::new(
            "pre_commit_hooks",
            "Pre-commit hooks configured (.pre-commit-config.yaml, .husky/, lefthook) to enforce standards",
            gov.has_pre_commit,
            15,
            if !gov.has_pre_commit {
                Some("Add pre-commit hooks to enforce formatting and linting locally".to_string())
            } else {
                None
            },
        ),
        // Language-specific safety signal (Rust: unsafe audit; others: strict typing)
        MaturitySignal::new(
            "safety_config",
            "Language-specific safety configuration present (cargo-deny, #![forbid(unsafe_code)], TypeScript strict, mypy strict)",
            gov.has_safety_config,
            15,
            if !gov.has_safety_config {
                Some("Add a safety/audit config for your language (e.g. deny.toml for Rust, mypy strict for Python)".to_string())
            } else {
                None
            },
        ),
    ];
    DimensionScore::compute(MaturityDimension::CodeOrganization, signals)
}

/// Project Governance (15%) — CII Best Practices Passing + Silver level file checks.
fn score_project_governance(gov: &GovernanceReport) -> DimensionScore {
    let signals = vec![
        // LICENSE — CII floss_license + OpenSSF License check
        MaturitySignal::new(
            "license_present",
            "LICENSE, COPYING, or LICENSE-* file present at repository root",
            gov.has_license,
            30,
            if gov.license_all_rights_reserved {
                Some(
                    "No license file detected. Under copyright law this means \
                     All Rights Reserved — contributors and users have no rights \
                     to use, copy, modify, or distribute the code. Add a LICENSE \
                     file (MIT, Apache-2.0, or similar OSI-approved license)."
                        .to_string(),
                )
            } else {
                None
            },
        ),
        // README — self-descriptiveness (ISO 25010 Interaction Capability)
        MaturitySignal::new(
            "readme_present",
            "README file present describing the project",
            gov.has_readme,
            25,
            if !gov.has_readme {
                Some("Add a README.md describing what the project does and how to use it".to_string())
            } else {
                None
            },
        ),
        // CHANGELOG — CII release_notes criterion
        MaturitySignal::new(
            "changelog_present",
            "CHANGELOG (or CHANGES/HISTORY) file present documenting version history",
            gov.has_changelog,
            20,
            if !gov.has_changelog {
                Some("Add a CHANGELOG.md following Keep a Changelog format (keepachangelog.com)".to_string())
            } else {
                None
            },
        ),
        // CONTRIBUTING — CII contribution criterion
        MaturitySignal::new(
            "contributing_present",
            "CONTRIBUTING.md present explaining how to contribute",
            gov.has_contributing,
            15,
            if !gov.has_contributing {
                Some("Add CONTRIBUTING.md with development setup, PR guidelines, and code standards".to_string())
            } else {
                None
            },
        ),
        // Code of Conduct — CII Silver code_of_conduct criterion
        MaturitySignal::new(
            "code_of_conduct",
            "CODE_OF_CONDUCT.md present establishing community standards",
            gov.has_code_of_conduct,
            10,
            if !gov.has_code_of_conduct {
                Some("Add CODE_OF_CONDUCT.md (Contributor Covenant is widely adopted)".to_string())
            } else {
                None
            },
        ),
    ];
    DimensionScore::compute(MaturityDimension::ProjectGovernance, signals)
}

/// Testing & Quality (10%) — CII test + SAST criteria; ISO 25010 Testability.
fn score_testing_and_quality(gov: &GovernanceReport) -> DimensionScore {
    let signals = vec![
        // Test directory or files present — CII test criterion
        MaturitySignal::new(
            "test_files_present",
            "Test files or test directory present (tests/, spec/, *_test.*, *.test.*)",
            gov.has_test_files,
            45,
            if !gov.has_test_files {
                Some("Add a test suite appropriate for your language (cargo test, pytest, jest, go test, etc.)".to_string())
            } else {
                None
            },
        ),
        // CI runs tests — CII test_continuous_integration criterion
        MaturitySignal::new(
            "ci_runs_tests",
            "CI configuration is present and presumably runs tests",
            gov.has_ci_config,
            35,
            if !gov.has_ci_config {
                Some("Add CI that runs tests on every push/PR".to_string())
            } else {
                None
            },
        ),
        // SAST/linter configured — CII static_analysis + OpenSSF SAST (static portion)
        MaturitySignal::new(
            "sast_configured",
            "Static analysis or linter tool configured in the repository",
            gov.has_lint_config,
            20,
            if !gov.has_lint_config {
                Some("Configure a SAST or linter tool (clippy, ESLint, ruff, golangci-lint, etc.)".to_string())
            } else {
                None
            },
        ),
    ];
    DimensionScore::compute(MaturityDimension::TestingAndQuality, signals)
}
