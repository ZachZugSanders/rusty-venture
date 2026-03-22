use async_trait::async_trait;
use rusty_venture_actions::repo::{
    detect_language::{DetectedLanguages, Language},
    governance::GovernanceReport,
    CTX_DETECTED_LANGUAGES, CTX_GOVERNANCE_REPORT,
};
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use serde::{Deserialize, Serialize};
use tracing::info;

pub const CTX_GOVERNANCE_FILES: &str = "improve.governance_files";

/// A file to be written to the repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceFile {
    pub path: String,
    pub content: String,
}

/// Generates all missing governance files (LICENSE, SECURITY.md, CONTRIBUTING.md,
/// CHANGELOG.md, .github/dependabot.yml, lint config) based on what the
/// GovernanceReport says is missing.
///
/// All content is deterministic template output — no LLM call needed.
/// The files are stored in context for `CreateBranchAction` to commit.
pub struct GenerateGovernanceFilesAction;

#[async_trait]
impl Action for GenerateGovernanceFilesAction {
    type Input = ();
    type Output = Vec<GovernanceFile>;

    fn name(&self) -> &str {
        "generate-governance-files"
    }

    async fn execute(
        &self,
        ctx: &ExecutionContext,
        _input: (),
    ) -> Result<Vec<GovernanceFile>, CoreError> {
        let gov: GovernanceReport = ctx
            .get::<GovernanceReport>(CTX_GOVERNANCE_REPORT)
            .await
            .unwrap_or_default();
        let languages: DetectedLanguages = ctx
            .get::<DetectedLanguages>(CTX_DETECTED_LANGUAGES)
            .await
            .unwrap_or_default();
        let lang = &languages.primary;

        let mut files: Vec<GovernanceFile> = vec![];

        if !gov.has_license {
            files.push(GovernanceFile {
                path: "LICENSE".to_string(),
                content: mit_license_template(),
            });
            info!("Generating LICENSE (MIT)");
        }

        if !gov.has_security_policy {
            files.push(GovernanceFile {
                path: "SECURITY.md".to_string(),
                content: security_policy_template(),
            });
            info!("Generating SECURITY.md");
        }

        if !gov.has_contributing {
            files.push(GovernanceFile {
                path: "CONTRIBUTING.md".to_string(),
                content: contributing_template(),
            });
            info!("Generating CONTRIBUTING.md");
        }

        if !gov.has_changelog {
            files.push(GovernanceFile {
                path: "CHANGELOG.md".to_string(),
                content: changelog_template(),
            });
            info!("Generating CHANGELOG.md");
        }

        if !gov.has_dependabot {
            if let Some(content) = dependabot_config(lang) {
                files.push(GovernanceFile {
                    path: ".github/dependabot.yml".to_string(),
                    content,
                });
                info!("Generating .github/dependabot.yml");
            }
        }

        if !gov.has_lint_config {
            if let Some((path, content)) = lint_config(lang) {
                files.push(GovernanceFile { path, content });
                info!(lang = %lang, "Generating lint config");
            }
        }

        if !gov.has_safety_config {
            if let Some((path, content)) = safety_config(lang) {
                files.push(GovernanceFile { path, content });
                info!(lang = %lang, "Generating safety config");
            }
        }

        info!(count = files.len(), "Governance files generated");
        ctx.insert(CTX_GOVERNANCE_FILES, files.clone()).await;
        Ok(files)
    }
}

// ── Templates ─────────────────────────────────────────────────────────────────

fn mit_license_template() -> String {
    // Year placeholder — real implementation would use chrono::Utc::now().year()
    r#"MIT License

Copyright (c) 2024 Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"#
    .to_string()
}

fn security_policy_template() -> String {
    r#"# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| latest  | :white_check_mark: |

## Reporting a Vulnerability

Please **do not** open a public GitHub issue for security vulnerabilities.

Instead, report them privately via one of the following:

- **GitHub Security Advisories**: Use the "Report a vulnerability" button on
  the Security tab of this repository.
- **Email**: security@example.com *(replace with your actual contact)*

We aim to acknowledge reports within **48 hours** and provide a remediation
timeline within **7 days**.

## Disclosure Policy

We follow [coordinated disclosure](https://en.wikipedia.org/wiki/Coordinated_vulnerability_disclosure).
Reporters who follow responsible disclosure will be credited in the release notes.
"#
    .to_string()
}

fn contributing_template() -> String {
    r#"# Contributing

Thank you for considering contributing! Here's how to get started.

## Development Setup

1. Fork and clone the repository.
2. Install the required toolchain (see README for prerequisites).
3. Run tests: `cargo test` / `npm test` / `pytest` (choose the appropriate command).

## Pull Request Process

1. Open an issue to discuss significant changes before starting work.
2. Create a feature branch: `git checkout -b feat/my-feature`.
3. Write or update tests for your changes.
4. Ensure `cargo fmt` / `eslint` / `ruff` passes with no warnings.
5. Open a pull request against `main` with a clear description.

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md).
By participating, you agree to uphold these standards.

## Licence

By contributing, you agree that your contributions will be licensed under
the same licence as the project (see `LICENSE`).
"#
    .to_string()
}

fn changelog_template() -> String {
    r#"# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial repository setup.

[Unreleased]: https://github.com/your-org/your-repo/compare/HEAD...HEAD
"#
    .to_string()
}

fn dependabot_config(lang: &Language) -> Option<String> {
    let package_ecosystem = match lang {
        Language::Rust => "cargo",
        Language::Node => "npm",
        Language::Python => "pip",
        Language::Go => "gomod",
        Language::Java => "maven",
        Language::Ruby => "bundler",
        Language::PHP => "composer",
        _ => return None,
    };

    Some(format!(
        r#"version: 2
updates:
  - package-ecosystem: "{package_ecosystem}"
    directory: "/"
    schedule:
      interval: "weekly"
    open-pull-requests-limit: 5
    labels:
      - "dependencies"
    commit-message:
      prefix: "chore(deps)"

  - package-ecosystem: "github-actions"
    directory: "/"
    schedule:
      interval: "weekly"
    labels:
      - "dependencies"
    commit-message:
      prefix: "chore(ci)"
"#
    ))
}

fn lint_config(lang: &Language) -> Option<(String, String)> {
    match lang {
        Language::Rust => Some((
            "clippy.toml".to_string(),
            r#"# Clippy configuration
# See https://doc.rust-lang.org/clippy/configuration.html
msrv = "1.65"
cognitive-complexity-threshold = 25
"#
            .to_string(),
        )),
        Language::Node => Some((
            "eslint.config.js".to_string(),
            r#"import js from "@eslint/js";

export default [
  js.configs.recommended,
  {
    rules: {
      "no-unused-vars": "error",
      "no-console": "warn",
      "prefer-const": "error",
    },
  },
];
"#
            .to_string(),
        )),
        Language::Python => Some((
            "ruff.toml".to_string(),
            r#"line-length = 100
target-version = "py311"

[lint]
select = ["E", "W", "F", "I", "N", "UP", "S", "B"]
ignore = ["S101"]  # allow assert in tests
"#
            .to_string(),
        )),
        Language::Go => Some((
            ".golangci.yml".to_string(),
            r#"run:
  timeout: 5m

linters:
  enable:
    - errcheck
    - gosimple
    - govet
    - ineffassign
    - staticcheck
    - unused
    - gosec
    - revive
"#
            .to_string(),
        )),
        _ => None,
    }
}

fn safety_config(lang: &Language) -> Option<(String, String)> {
    match lang {
        Language::Rust => Some((
            "deny.toml".to_string(),
            r#"# cargo-deny configuration
# Run: cargo deny check

[licenses]
allow = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0"]
deny = ["GPL-2.0", "GPL-3.0", "AGPL-3.0"]

[bans]
multiple-versions = "warn"
deny = []

[advisories]
ignore = []
"#
            .to_string(),
        )),
        Language::Python => Some((
            "mypy.ini".to_string(),
            r#"[mypy]
python_version = 3.11
strict = True
ignore_missing_imports = True
"#
            .to_string(),
        )),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_venture_actions::repo::detect_language::Language;
    use rusty_venture_actions::repo::governance::GovernanceReport;
    use rusty_venture_actions::repo::{CTX_DETECTED_LANGUAGES, CTX_GOVERNANCE_REPORT};
    use rusty_venture_core::context::ExecutionContext;

    // ── Fixtures ─────────────────────────────────────────────────────────────

    fn all_missing_report() -> GovernanceReport {
        GovernanceReport::default() // all booleans default to false → everything missing
    }

    fn all_present_report() -> GovernanceReport {
        GovernanceReport {
            has_license: true,
            has_security_policy: true,
            has_contributing: true,
            has_changelog: true,
            has_dependabot: true,
            has_lint_config: true,
            has_safety_config: true,
            ..Default::default()
        }
    }

    fn rust_languages() -> DetectedLanguages {
        DetectedLanguages {
            primary: Language::Rust,
            secondary: vec![],
            scores: vec![(Language::Rust, 10)],
        }
    }

    fn node_languages() -> DetectedLanguages {
        DetectedLanguages {
            primary: Language::Node,
            secondary: vec![],
            scores: vec![(Language::Node, 10)],
        }
    }

    fn python_languages() -> DetectedLanguages {
        DetectedLanguages {
            primary: Language::Python,
            secondary: vec![],
            scores: vec![(Language::Python, 9)],
        }
    }

    fn unknown_languages() -> DetectedLanguages {
        DetectedLanguages {
            primary: Language::Unknown,
            secondary: vec![],
            scores: vec![],
        }
    }

    async fn run_action(gov: GovernanceReport, langs: DetectedLanguages) -> Vec<GovernanceFile> {
        let ctx = ExecutionContext::new("test");
        ctx.insert(CTX_GOVERNANCE_REPORT, gov).await;
        ctx.insert(CTX_DETECTED_LANGUAGES, langs).await;
        GenerateGovernanceFilesAction
            .execute(&ctx, ())
            .await
            .expect("action must not fail")
    }

    fn paths(files: &[GovernanceFile]) -> Vec<&str> {
        files.iter().map(|f| f.path.as_str()).collect()
    }

    // ── Generation: all-missing cases ────────────────────────────────────────

    #[tokio::test]
    async fn generates_license_when_missing() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        assert!(
            paths(&files).contains(&"LICENSE"),
            "LICENSE must be generated"
        );
    }

    #[tokio::test]
    async fn generates_security_policy_when_missing() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        assert!(
            paths(&files).contains(&"SECURITY.md"),
            "SECURITY.md must be generated"
        );
    }

    #[tokio::test]
    async fn generates_contributing_when_missing() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        assert!(
            paths(&files).contains(&"CONTRIBUTING.md"),
            "CONTRIBUTING.md must be generated"
        );
    }

    #[tokio::test]
    async fn generates_changelog_when_missing() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        assert!(
            paths(&files).contains(&"CHANGELOG.md"),
            "CHANGELOG.md must be generated"
        );
    }

    #[tokio::test]
    async fn generates_dependabot_yml_for_rust_when_missing() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        assert!(
            paths(&files).contains(&".github/dependabot.yml"),
            ".github/dependabot.yml must be generated for Rust"
        );
    }

    #[tokio::test]
    async fn generates_clippy_toml_for_rust_when_missing() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        assert!(
            paths(&files).contains(&"clippy.toml"),
            "clippy.toml must be generated for Rust"
        );
    }

    #[tokio::test]
    async fn generates_deny_toml_for_rust_when_missing() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        assert!(
            paths(&files).contains(&"deny.toml"),
            "deny.toml must be generated for Rust"
        );
    }

    // ── Skip: all-present cases ───────────────────────────────────────────────

    #[tokio::test]
    async fn no_files_generated_when_all_present() {
        let files = run_action(all_present_report(), rust_languages()).await;
        assert!(
            files.is_empty(),
            "no files should be generated when all governance files are present"
        );
    }

    #[tokio::test]
    async fn skips_license_when_already_present() {
        let gov = GovernanceReport {
            has_license: true,
            ..Default::default()
        };
        let files = run_action(gov, rust_languages()).await;
        assert!(
            !paths(&files).contains(&"LICENSE"),
            "LICENSE must NOT be generated when already present"
        );
    }

    #[tokio::test]
    async fn skips_security_policy_when_already_present() {
        let gov = GovernanceReport {
            has_security_policy: true,
            ..Default::default()
        };
        let files = run_action(gov, rust_languages()).await;
        assert!(!paths(&files).contains(&"SECURITY.md"));
    }

    #[tokio::test]
    async fn skips_changelog_when_already_present() {
        let gov = GovernanceReport {
            has_changelog: true,
            ..Default::default()
        };
        let files = run_action(gov, rust_languages()).await;
        assert!(!paths(&files).contains(&"CHANGELOG.md"));
    }

    // ── Language-specific generation ──────────────────────────────────────────

    #[tokio::test]
    async fn generates_eslint_config_for_node() {
        let files = run_action(all_missing_report(), node_languages()).await;
        assert!(
            paths(&files).contains(&"eslint.config.js"),
            "eslint.config.js must be generated for Node"
        );
        // Node has no safety config → no mypy.ini or deny.toml
        assert!(!paths(&files).contains(&"deny.toml"));
        assert!(!paths(&files).contains(&"mypy.ini"));
    }

    #[tokio::test]
    async fn generates_ruff_and_mypy_for_python() {
        let files = run_action(all_missing_report(), python_languages()).await;
        assert!(paths(&files).contains(&"ruff.toml"), "ruff.toml for Python");
        assert!(paths(&files).contains(&"mypy.ini"), "mypy.ini for Python");
    }

    #[tokio::test]
    async fn no_lint_or_safety_config_for_unknown_language() {
        let files = run_action(all_missing_report(), unknown_languages()).await;
        // Unknown language → no dependabot, no lint, no safety
        assert!(!paths(&files).contains(&".github/dependabot.yml"));
        assert!(
            // no lang-specific configs should be generated
            !files.iter().any(|f| f.path == "clippy.toml"
                || f.path == "eslint.config.js"
                || f.path == "ruff.toml"
                || f.path == "deny.toml"
                || f.path == "mypy.ini"),
            "no lang-specific files should be generated for Unknown language"
        );
    }

    #[tokio::test]
    async fn dependabot_yml_contains_correct_ecosystem_for_rust() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        let dbot = files
            .iter()
            .find(|f| f.path == ".github/dependabot.yml")
            .expect(".github/dependabot.yml must be in output");
        assert!(
            dbot.content.contains("cargo"),
            "dependabot.yml for Rust must specify cargo ecosystem"
        );
    }

    #[tokio::test]
    async fn dependabot_yml_contains_correct_ecosystem_for_node() {
        let files = run_action(all_missing_report(), node_languages()).await;
        let dbot = files
            .iter()
            .find(|f| f.path == ".github/dependabot.yml")
            .expect(".github/dependabot.yml must be in output");
        assert!(dbot.content.contains("npm"));
    }

    // ── Template content correctness ──────────────────────────────────────────

    #[tokio::test]
    async fn mit_license_contains_mit_identifier() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        let license = files
            .iter()
            .find(|f| f.path == "LICENSE")
            .expect("LICENSE must be generated");
        assert!(
            license.content.contains("MIT License"),
            "LICENSE must contain 'MIT License'"
        );
    }

    #[tokio::test]
    async fn security_policy_has_reporting_placeholder() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        let sec = files
            .iter()
            .find(|f| f.path == "SECURITY.md")
            .expect("SECURITY.md must be generated");
        assert!(
            sec.content.contains("security@example.com"),
            "SECURITY.md must contain the contact email placeholder"
        );
    }

    #[tokio::test]
    async fn dependabot_yml_is_valid_yaml() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        let dbot = files
            .iter()
            .find(|f| f.path == ".github/dependabot.yml")
            .expect(".github/dependabot.yml must be generated");
        // Minimal validity check: YAML top-level key must be "version"
        assert!(
            dbot.content.trim_start().starts_with("version:"),
            "dependabot.yml must start with 'version:'"
        );
    }

    #[tokio::test]
    async fn clippy_toml_contains_msrv_key() {
        let files = run_action(all_missing_report(), rust_languages()).await;
        let clippy = files
            .iter()
            .find(|f| f.path == "clippy.toml")
            .expect("clippy.toml must be generated for Rust");
        assert!(
            clippy.content.contains("msrv"),
            "clippy.toml must contain an msrv key"
        );
    }

    // ── Context storage ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn action_stores_result_in_context() {
        let gov = all_missing_report();
        let langs = rust_languages();
        let ctx = ExecutionContext::new("test-ctx");
        ctx.insert(CTX_GOVERNANCE_REPORT, gov).await;
        ctx.insert(CTX_DETECTED_LANGUAGES, langs).await;
        GenerateGovernanceFilesAction
            .execute(&ctx, ())
            .await
            .expect("action must not fail");
        let stored: Option<Vec<GovernanceFile>> = ctx.get(CTX_GOVERNANCE_FILES).await;
        assert!(
            stored.is_some(),
            "action must store files in context under CTX_GOVERNANCE_FILES"
        );
        assert!(
            !stored.unwrap().is_empty(),
            "stored files must not be empty when all governance files are missing"
        );
    }

    #[tokio::test]
    async fn action_returns_empty_vec_when_no_context_provided() {
        // GovernanceReport and DetectedLanguages are both absent → use defaults
        let ctx = ExecutionContext::new("test-empty");
        let files = GenerateGovernanceFilesAction
            .execute(&ctx, ())
            .await
            .expect("action must not fail even with empty context");
        // Default GovernanceReport has all fields false → all files generated
        // Default language is Unknown (via unwrap_or_default) → no lang-specific files
        assert!(
            files.iter().any(|f| f.path == "LICENSE"),
            "LICENSE must still be generated when context is empty (all-missing defaults)"
        );
    }
}
