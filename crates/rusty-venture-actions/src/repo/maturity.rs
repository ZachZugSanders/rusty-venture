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
    active_validation::ActiveValidationReport, analyze_deps::DependencyReport,
    audit_files::AuditReport, content_quality::ContentQualityReport, detect_language::Language,
    find_dockerfiles::DockerfileReport, governance::GovernanceReport, report::FinalReport,
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
    /// Maturity tier this signal belongs to:
    ///   1 = Static Discovery (file presence / static checks)
    ///   2 = Content Quality  (file content inspection)
    ///   3 = Active Functional Validation (run commands, execute tests)
    pub tier: u8,
}

impl MaturitySignal {
    /// Construct a Tier 1 (Static Discovery) signal.
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
            tier: 1,
        }
    }

    /// Construct a signal for an explicit tier.
    #[allow(dead_code)]
    fn new_tier(
        name: impl Into<String>,
        description: impl Into<String>,
        passed: bool,
        points: u8,
        detail: impl Into<Option<String>>,
        tier: u8,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            passed,
            points,
            detail: detail.into(),
            tier,
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
        let earned: u32 = signals
            .iter()
            .filter(|s| s.passed)
            .map(|s| s.points as u32)
            .sum();
        let score = if total == 0 {
            0u8
        } else {
            ((earned as f32 / total as f32) * 100.0).round() as u8
        };
        Self {
            dimension,
            score,
            signals,
        }
    }
}

/// Overall maturity grade expressed as a league tier.
///
/// Scores map to tiers as follows:
///   0–20  → Bronze   21–40 → Silver   41–60 → Gold
///   61–80 → Platinum 81–100 → Diamond
///
/// The `#[serde(rename_all = "SCREAMING_SNAKE_CASE")]` attribute ensures that
/// the JSON emitted by the `/analyze` API uses the same all-caps vocabulary
/// as the `maturity_grade` column stored in the database (e.g. `"DIAMOND"`),
/// eliminating the previous display mismatch between the live-result banner
/// and the history/repos tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MaturityGrade {
    /// 0–20: Ad hoc processes, minimal hygiene.
    Bronze,
    /// 21–40: Some practices in place but inconsistent.
    Silver,
    /// 41–60: Most core practices present; meaningful gaps remain.
    Gold,
    /// 61–80: Consistently applied practices; minor gaps.
    Platinum,
    /// 81–100: Exemplary hygiene across all dimensions.
    Diamond,
}

impl MaturityGrade {
    pub fn from_score(score: u8) -> Self {
        match score {
            0..=20 => Self::Bronze,
            21..=40 => Self::Silver,
            41..=60 => Self::Gold,
            61..=80 => Self::Platinum,
            _ => Self::Diamond,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Bronze => "BRONZE",
            Self::Silver => "SILVER",
            Self::Gold => "GOLD",
            Self::Platinum => "PLATINUM",
            Self::Diamond => "DIAMOND",
        }
    }

    /// Map a historical or current label string to a `MaturityGrade`.
    ///
    /// Accepts both the current BRONZE…DIAMOND league-tier vocabulary and the
    /// legacy NASCENT…EXEMPLARY strings present in older database rows, so that
    /// existing scan history remains fully readable after the rename migration.
    /// Returns `None` for any unrecognised string (case-insensitive).
    pub fn from_legacy_label(label: &str) -> Option<Self> {
        match label.to_ascii_uppercase().as_str() {
            // Current league tiers
            "BRONZE" => Some(Self::Bronze),
            "SILVER" => Some(Self::Silver),
            "GOLD" => Some(Self::Gold),
            "PLATINUM" => Some(Self::Platinum),
            "DIAMOND" => Some(Self::Diamond),
            // Legacy maturity-stage labels — each maps to its equivalent tier
            "NASCENT" => Some(Self::Bronze),
            "EMERGING" => Some(Self::Silver),
            "DEVELOPING" => Some(Self::Gold),
            "ESTABLISHED" => Some(Self::Platinum),
            "EXEMPLARY" => Some(Self::Diamond),
            _ => None,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- Tier boundary mapping ------------------------------------------------

    /// Every boundary score listed in the issue must land in the correct tier.
    #[test]
    fn tier_boundaries_at_floor_and_ceiling_of_each_band() {
        assert_eq!(MaturityGrade::from_score(0), MaturityGrade::Bronze);
        assert_eq!(MaturityGrade::from_score(20), MaturityGrade::Bronze);
        assert_eq!(MaturityGrade::from_score(21), MaturityGrade::Silver);
        assert_eq!(MaturityGrade::from_score(40), MaturityGrade::Silver);
        assert_eq!(MaturityGrade::from_score(41), MaturityGrade::Gold);
        assert_eq!(MaturityGrade::from_score(60), MaturityGrade::Gold);
        assert_eq!(MaturityGrade::from_score(61), MaturityGrade::Platinum);
        assert_eq!(MaturityGrade::from_score(80), MaturityGrade::Platinum);
        assert_eq!(MaturityGrade::from_score(81), MaturityGrade::Diamond);
        assert_eq!(MaturityGrade::from_score(100), MaturityGrade::Diamond);
    }

    // -- Label strings are league tiers --------------------------------------

    #[test]
    fn labels_are_league_tier_vocabulary() {
        const LEAGUE_TIERS: &[&str] = &["BRONZE", "SILVER", "GOLD", "PLATINUM", "DIAMOND"];
        for grade in [
            MaturityGrade::Bronze,
            MaturityGrade::Silver,
            MaturityGrade::Gold,
            MaturityGrade::Platinum,
            MaturityGrade::Diamond,
        ] {
            assert!(
                LEAGUE_TIERS.contains(&grade.label()),
                "{:?} emits non-league-tier label: {:?}",
                grade,
                grade.label()
            );
        }
    }

    #[test]
    fn no_legacy_or_abcdf_labels_in_active_grade_logic() {
        const FORBIDDEN: &[&str] = &[
            "NASCENT",
            "EMERGING",
            "DEVELOPING",
            "ESTABLISHED",
            "EXEMPLARY",
            "A",
            "B",
            "C",
            "D",
            "F",
        ];
        for grade in [
            MaturityGrade::Bronze,
            MaturityGrade::Silver,
            MaturityGrade::Gold,
            MaturityGrade::Platinum,
            MaturityGrade::Diamond,
        ] {
            assert!(
                !FORBIDDEN.contains(&grade.label()),
                "{:?} uses forbidden label: {:?}",
                grade,
                grade.label()
            );
        }
    }

    // -- Legacy label compatibility -------------------------------------------

    #[test]
    fn from_legacy_label_maps_all_historical_values() {
        assert_eq!(
            MaturityGrade::from_legacy_label("NASCENT"),
            Some(MaturityGrade::Bronze)
        );
        assert_eq!(
            MaturityGrade::from_legacy_label("EMERGING"),
            Some(MaturityGrade::Silver)
        );
        assert_eq!(
            MaturityGrade::from_legacy_label("DEVELOPING"),
            Some(MaturityGrade::Gold)
        );
        assert_eq!(
            MaturityGrade::from_legacy_label("ESTABLISHED"),
            Some(MaturityGrade::Platinum)
        );
        assert_eq!(
            MaturityGrade::from_legacy_label("EXEMPLARY"),
            Some(MaturityGrade::Diamond)
        );
    }

    #[test]
    fn from_legacy_label_passes_through_current_tier_names() {
        assert_eq!(
            MaturityGrade::from_legacy_label("BRONZE"),
            Some(MaturityGrade::Bronze)
        );
        assert_eq!(
            MaturityGrade::from_legacy_label("SILVER"),
            Some(MaturityGrade::Silver)
        );
        assert_eq!(
            MaturityGrade::from_legacy_label("GOLD"),
            Some(MaturityGrade::Gold)
        );
        assert_eq!(
            MaturityGrade::from_legacy_label("PLATINUM"),
            Some(MaturityGrade::Platinum)
        );
        assert_eq!(
            MaturityGrade::from_legacy_label("DIAMOND"),
            Some(MaturityGrade::Diamond)
        );
    }

    #[test]
    fn from_legacy_label_is_case_insensitive() {
        assert_eq!(
            MaturityGrade::from_legacy_label("bronze"),
            Some(MaturityGrade::Bronze)
        );
        assert_eq!(
            MaturityGrade::from_legacy_label("nascent"),
            Some(MaturityGrade::Bronze)
        );
        assert_eq!(
            MaturityGrade::from_legacy_label("Diamond"),
            Some(MaturityGrade::Diamond)
        );
    }

    #[test]
    fn from_legacy_label_returns_none_for_unknown() {
        assert_eq!(MaturityGrade::from_legacy_label("UNKNOWN"), None);
        assert_eq!(MaturityGrade::from_legacy_label(""), None);
        assert_eq!(MaturityGrade::from_legacy_label("A"), None);
        assert_eq!(MaturityGrade::from_legacy_label("D"), None);
    }

    // -- API contract: serialised grade is a league tier ----------------------
    //
    // These tests validate the invariant that the JSON emitted by POST /analyze
    // uses the league-tier vocabulary — no legacy strings, no A/B/C/D/F grades.

    #[test]
    fn maturity_grade_serialises_to_screaming_snake_league_tier() {
        let cases = [
            (MaturityGrade::Bronze, "\"BRONZE\""),
            (MaturityGrade::Silver, "\"SILVER\""),
            (MaturityGrade::Gold, "\"GOLD\""),
            (MaturityGrade::Platinum, "\"PLATINUM\""),
            (MaturityGrade::Diamond, "\"DIAMOND\""),
        ];
        for (grade, expected_json) in cases {
            let json = serde_json::to_string(&grade).expect("serialize MaturityGrade");
            assert_eq!(
                json, expected_json,
                "{:?} did not serialise to {:?}",
                grade, expected_json
            );
        }
    }

    #[test]
    fn maturity_grade_deserialises_only_league_tier_strings() {
        // Current tier strings must round-trip.
        let valid = [
            ("\"BRONZE\"", MaturityGrade::Bronze),
            ("\"SILVER\"", MaturityGrade::Silver),
            ("\"GOLD\"", MaturityGrade::Gold),
            ("\"PLATINUM\"", MaturityGrade::Platinum),
            ("\"DIAMOND\"", MaturityGrade::Diamond),
        ];
        for (json, expected) in valid {
            let grade: MaturityGrade =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("deserialize {json:?}: {e}"));
            assert_eq!(grade, expected);
        }

        // Legacy and A/B/C/D/F strings must NOT deserialise.
        let forbidden = [
            "\"NASCENT\"",
            "\"EMERGING\"",
            "\"DEVELOPING\"",
            "\"ESTABLISHED\"",
            "\"EXEMPLARY\"",
            "\"A\"",
            "\"B\"",
            "\"C\"",
            "\"D\"",
            "\"F\"",
        ];
        for s in forbidden {
            let result: Result<MaturityGrade, _> = serde_json::from_str(s);
            assert!(
                result.is_err(),
                "Forbidden value {s} should not deserialise as MaturityGrade"
            );
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
/// `content` is `Some` only when `scan_tier >= 2` and the content-quality
/// action ran successfully. When `None`, Tier 2 signals are omitted from all
/// dimensions so the score is based purely on the Tier 1 static checks.
#[allow(clippy::too_many_arguments)]
pub fn compute_maturity(
    _report: &FinalReport,
    deps: &DependencyReport,
    dockerfiles: &DockerfileReport,
    audit: &AuditReport,
    primary_language: &Language,
    gov: &GovernanceReport,
    content: Option<&ContentQualityReport>,
    active: Option<&ActiveValidationReport>,
) -> MaturityScore {
    let dimensions = vec![
        score_security(audit, gov, content),
        score_dependency_health(deps, dockerfiles, gov, content),
        score_build_and_ci(dockerfiles, gov, content, active),
        score_code_organization(primary_language, deps, gov, active),
        score_project_governance(gov, content),
        score_testing_and_quality(gov, content, active),
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
fn score_security(
    audit: &AuditReport,
    gov: &GovernanceReport,
    content: Option<&ContentQualityReport>,
) -> DimensionScore {
    let mut signals = vec![
        // No critical violations — highest weight; directly maps to OpenSSF Binary-Artifacts
        // and CII no_leaked_credentials criteria.
        MaturitySignal::new(
            "no_critical_violations",
            "No critical committed files (secrets, credentials, private keys)",
            audit.critical_count == 0,
            40,
            if audit.critical_count > 0 {
                Some(format!(
                    "{} critical violation(s) found",
                    audit.critical_count
                ))
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
                Some(format!(
                    "{} warning violation(s) found",
                    audit.warning_count
                ))
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

    // ── Tier 2 signals ───────────────────────────────────────────────────────
    if let Some(cq) = content {
        // License is a recognised SPDX identifier — confirms code is actually open-source
        signals.push(MaturitySignal::new_tier(
            "license_spdx_recognized",
            "LICENSE file contains a recognised SPDX license identifier (MIT, Apache, GPL, BSD, etc.)",
            cq.license_is_spdx,
            20,
            if !cq.license_is_spdx {
                Some("LICENSE file does not contain a recognised SPDX identifier. Use a standard license text (MIT, Apache-2.0, etc.)".to_string())
            } else {
                None
            },
            2,
        ));
    }

    DimensionScore::compute(MaturityDimension::Security, signals)
}

/// Dependency Health (20%) — CII external_dependencies + lock file + OpenSSF Pinned-Dependencies.
fn score_dependency_health(
    deps: &DependencyReport,
    dockerfiles: &DockerfileReport,
    gov: &GovernanceReport,
    content: Option<&ContentQualityReport>,
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
        Some(format!(
            "{} direct dependencies — consider auditing for unused/redundant ones",
            dep_count
        ))
    } else {
        None
    };

    let mut signals = vec![
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

    // ── Tier 2 signals ───────────────────────────────────────────────────────
    if let Some(cq) = content {
        // Lock file freshness — lock file is not older than the dependency manifest
        if cq.lockfile_pair_found {
            signals.push(MaturitySignal::new_tier(
                "lockfile_not_stale",
                "Lock file is up to date relative to the dependency manifest",
                cq.lockfile_not_stale,
                25,
                if !cq.lockfile_not_stale {
                    Some("Lock file appears stale — run your package manager to regenerate it (cargo update, npm install, etc.)".to_string())
                } else {
                    None
                },
                2,
            ));
        }
    }

    DimensionScore::compute(MaturityDimension::DependencyHealth, signals)
}

/// Build & CI (15%) — OpenSSF Dangerous-Workflow (static) + CII CI criteria.
fn score_build_and_ci(
    dockerfiles: &DockerfileReport,
    gov: &GovernanceReport,
    content: Option<&ContentQualityReport>,
    active: Option<&ActiveValidationReport>,
) -> DimensionScore {
    let has_dockerfile_issues = dockerfiles.entries.iter().any(|e| !e.warnings.is_empty());

    let mut signals = vec![
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

    // ── Tier 2 signals ───────────────────────────────────────────────────────
    if let Some(cq) = content {
        // CI config actually references a test command (not just presence check)
        if gov.has_ci_config {
            signals.push(MaturitySignal::new_tier(
                "ci_runs_tests_command",
                "CI configuration references an actual test command (cargo test, pytest, jest, etc.)",
                cq.ci_has_test_command,
                30,
                if !cq.ci_has_test_command {
                    Some("CI config found but no test command detected — add a test step to your CI pipeline".to_string())
                } else {
                    None
                },
                2,
            ));
        }

        // Dockerfile switches to a non-root user
        if dockerfiles.has_root_dockerfile {
            signals.push(MaturitySignal::new_tier(
                "dockerfile_nonroot_user",
                "Dockerfile switches to a non-root user with a USER directive",
                cq.dockerfile_has_nonroot_user,
                20,
                if !cq.dockerfile_has_nonroot_user {
                    Some("Add a USER directive in your Dockerfile to run as a non-root user (e.g. USER appuser)".to_string())
                } else {
                    None
                },
                2,
            ));
        }
    }

    // ── Tier 3 signals ───────────────────────────────────────────────────────
    if let Some(av) = active {
        // Build succeeds
        if let Some(passed) = av.build_result {
            signals.push(MaturitySignal::new_tier(
                "build_succeeds",
                "Project builds successfully (cargo build, npm run build, go build, etc.)",
                passed,
                40,
                if passed {
                    if av.build_command.is_empty() {
                        None
                    } else {
                        Some(format!("`{}` succeeded", av.build_command))
                    }
                } else {
                    Some(format!(
                        "`{}` failed — snippet: {}",
                        av.build_command,
                        av.build_output_snippet
                            .chars()
                            .take(200)
                            .collect::<String>()
                    ))
                },
                3,
            ));
        }

        // Docker Compose YAML is valid
        if let Some(valid) = av.compose_result {
            signals.push(MaturitySignal::new_tier(
                "docker_compose_valid",
                "docker-compose.yml / compose.yml is valid YAML",
                valid,
                20,
                if !valid {
                    Some("Compose file contains YAML syntax errors".to_string())
                } else {
                    None
                },
                3,
            ));
        }
    }

    DimensionScore::compute(MaturityDimension::BuildAndCi, signals)
}

/// Code Organization (15%) — ISO 25010 Maintainability + language API guidelines.
fn score_code_organization(
    primary_language: &Language,
    deps: &DependencyReport,
    gov: &GovernanceReport,
    active: Option<&ActiveValidationReport>,
) -> DimensionScore {
    // Language-specific version constraint signal
    let has_version_constraint = match primary_language {
        Language::Rust => gov.rust_msrv_declared,
        Language::Node => gov.node_engine_declared,
        Language::Python => gov.python_requires_declared,
        Language::Go => gov.go_version_declared,
        Language::Java
        | Language::Ruby
        | Language::PHP
        | Language::CSharp
        | Language::Swift
        | Language::Kotlin => !deps.manifest_file.is_empty(),
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

    let mut signals = vec![
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

    // ── Tier 3 signals ───────────────────────────────────────────────────────
    if let Some(av) = active {
        if let Some(passed) = av.lint_result {
            signals.push(MaturitySignal::new_tier(
                "linter_passes",
                "Linter / static analysis passes with zero warnings or errors",
                passed,
                30,
                if passed {
                    Some(format!("`{}` passed with no issues", av.lint_command))
                } else {
                    Some(format!(
                        "`{}` reported issues — snippet: {}",
                        av.lint_command,
                        av.lint_output_snippet.chars().take(200).collect::<String>()
                    ))
                },
                3,
            ));
        }
    }

    DimensionScore::compute(MaturityDimension::CodeOrganization, signals)
}

/// Project Governance (15%) — CII Best Practices Passing + Silver level file checks.
fn score_project_governance(
    gov: &GovernanceReport,
    content: Option<&ContentQualityReport>,
) -> DimensionScore {
    let mut signals = vec![
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
                Some(
                    "Add a README.md describing what the project does and how to use it"
                        .to_string(),
                )
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
                Some(
                    "Add a CHANGELOG.md following Keep a Changelog format (keepachangelog.com)"
                        .to_string(),
                )
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
                Some(
                    "Add CONTRIBUTING.md with development setup, PR guidelines, and code standards"
                        .to_string(),
                )
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

    // ── Tier 2 signals ───────────────────────────────────────────────────────
    if let Some(cq) = content {
        // README has real substance (>100 words + section headings)
        if gov.has_readme {
            let readme_ok = cq.readme_word_count >= 100 && cq.readme_has_headings;
            signals.push(MaturitySignal::new_tier(
                "readme_has_substance",
                "README has meaningful content (≥100 words and section headings)",
                readme_ok,
                25,
                if !readme_ok {
                    let detail = if cq.readme_word_count < 100 {
                        format!("README has only {} words — expand it with installation, usage, and contribution instructions", cq.readme_word_count)
                    } else {
                        "README lacks section headings — add ## headings for Installation, Usage, etc.".to_string()
                    };
                    Some(detail)
                } else {
                    None
                },
                2,
            ));
        }

        // CHANGELOG has versioned entries
        if gov.has_changelog {
            signals.push(MaturitySignal::new_tier(
                "changelog_has_versions",
                "CHANGELOG contains versioned entries (semver format)",
                cq.changelog_has_versions,
                20,
                if !cq.changelog_has_versions {
                    Some("CHANGELOG found but no semver entries detected — use '## [1.0.0]' format (keepachangelog.com)".to_string())
                } else {
                    None
                },
                2,
            ));
        }
    }

    DimensionScore::compute(MaturityDimension::ProjectGovernance, signals)
}

/// Testing & Quality (10%) — CII test + SAST criteria; ISO 25010 Testability.
fn score_testing_and_quality(
    gov: &GovernanceReport,
    content: Option<&ContentQualityReport>,
    active: Option<&ActiveValidationReport>,
) -> DimensionScore {
    let mut signals = vec![
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
                Some(
                    "Configure a SAST or linter tool (clippy, ESLint, ruff, golangci-lint, etc.)"
                        .to_string(),
                )
            } else {
                None
            },
        ),
    ];

    // ── Tier 2 signals ───────────────────────────────────────────────────────
    if let Some(cq) = content {
        // Test files contain real test function definitions
        if gov.has_test_files {
            let has_real_tests = cq.test_function_count >= 1;
            signals.push(MaturitySignal::new_tier(
                "test_functions_present",
                "Test files contain actual test function definitions",
                has_real_tests,
                40,
                if !has_real_tests {
                    Some("Test files found but no test function definitions detected — add actual test functions (#[test], def test_*, func Test*, etc.)".to_string())
                } else {
                    Some(format!("{} test function(s) found", cq.test_function_count))
                },
                2,
            ));
        }
    }

    // ── Tier 3 signals ───────────────────────────────────────────────────────
    if let Some(av) = active {
        if let Some(passed) = av.test_result {
            signals.push(MaturitySignal::new_tier(
                "tests_pass",
                "Test suite runs and all tests pass",
                passed,
                60,
                if passed {
                    Some(format!("`{}` — all tests passed", av.test_command))
                } else {
                    Some(format!(
                        "`{}` had failures — snippet: {}",
                        av.test_command,
                        av.test_output_snippet.chars().take(200).collect::<String>()
                    ))
                },
                3,
            ));
        }
    }

    DimensionScore::compute(MaturityDimension::TestingAndQuality, signals)
}
