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
        let gov: GovernanceReport =
            ctx.get::<GovernanceReport>(CTX_GOVERNANCE_REPORT).await.unwrap_or_default();
        let languages: DetectedLanguages =
            ctx.get::<DetectedLanguages>(CTX_DETECTED_LANGUAGES).await.unwrap_or_default();
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
