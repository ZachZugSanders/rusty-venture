use std::collections::HashMap;

use async_trait::async_trait;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::container::{exec_in_container, ExecCommand, CTX_CONTAINER_ID, CTX_DOCKER_CLIENT};

pub const CTX_GOVERNANCE_REPORT: &str = "governance.report";

/// Signals gathered by checking file presence inside the analysis container.
/// All fields are derivable from a static repository scan with no external APIs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GovernanceReport {
    // ── Standard governance files ─────────────────────────────────────────
    pub has_license: bool,
    /// True when no license file was detected — implies "All Rights Reserved"
    /// under copyright law. Stored explicitly so reports and maturity signals
    /// can surface this clearly rather than just noting a missing file.
    pub license_all_rights_reserved: bool,
    pub has_readme: bool,
    pub has_changelog: bool,
    pub has_contributing: bool,
    pub has_security_policy: bool,
    pub has_code_of_conduct: bool,

    // ── CI & automation ───────────────────────────────────────────────────
    /// Any CI configuration file detected (GitHub Actions, GitLab CI,
    /// Travis CI, CircleCI, Jenkins, Bitbucket Pipelines).
    pub has_ci_config: bool,
    pub has_dependabot: bool,
    pub has_renovate: bool,

    // ── Dependency lock files (cross-language) ────────────────────────────
    /// True if any recognised lock file is present (used to supplement the
    /// per-language `DependencyReport.lock_file_present` field).
    pub has_any_lock_file: bool,

    // ── Code quality & safety tooling ─────────────────────────────────────
    /// Linter config: .clippy.toml, eslintrc.*, ruff.toml, golangci.yml, etc.
    pub has_lint_config: bool,
    /// Pre-commit hooks: .pre-commit-config.yaml, .husky/, lefthook.yml, etc.
    pub has_pre_commit: bool,
    /// Safety/audit config: deny.toml, #![forbid(unsafe_code)], mypy strict, etc.
    pub has_safety_config: bool,

    // ── Test infrastructure ───────────────────────────────────────────────
    /// Test directory or test files detected.
    pub has_test_files: bool,

    // ── Language-specific version constraint signals ───────────────────────
    /// Rust: `rust-version` field in Cargo.toml (MSRV declaration).
    pub rust_msrv_declared: bool,
    /// Node: `engines` field in package.json.
    pub node_engine_declared: bool,
    /// Python: `requires-python` in pyproject.toml.
    pub python_requires_declared: bool,
    /// Go: `go X.Y` directive present in go.mod.
    pub go_version_declared: bool,
}

/// Runs a single batch shell script in the analysis container and collects
/// file-presence signals needed for the maturity model.
///
/// All checks are purely static (file existence / grep); no code is executed.
pub struct GovernanceCheckAction;

#[async_trait]
impl Action for GovernanceCheckAction {
    type Input = ();
    type Output = GovernanceReport;

    fn name(&self) -> &str {
        "governance-check"
    }

    async fn execute(
        &self,
        ctx: &ExecutionContext,
        _input: (),
    ) -> Result<GovernanceReport, CoreError> {
        use std::sync::Arc;
        use bollard::Docker;

        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;

        let report = run_governance_script(&docker, &container_id).await?;

        info!(
            has_license = report.has_license,
            has_ci = report.has_ci_config,
            has_tests = report.has_test_files,
            has_security = report.has_security_policy,
            "Governance check complete"
        );

        ctx.insert(CTX_GOVERNANCE_REPORT, report.clone()).await;
        Ok(report)
    }
}

// ── Internal implementation ───────────────────────────────────────────────────

/// Shell script executed inside the container.
/// Outputs lines of the form `KEY=YES` or `KEY=NO`.
/// Uses only POSIX sh + coreutils available in both alpine and ubuntu.
const GOVERNANCE_SCRIPT: &str = r#"
R=/workspace/repo
YES=YES
NO=NO

c() {
  if [ -e "$R/$1" ]; then echo "$2=$YES"; else echo "$2=$NO"; fi
}
cdir() {
  if [ -d "$R/$1" ]; then echo "$2=$YES"; else echo "$2=$NO"; fi
}
cgrep() {
  if grep -qr "$1" "$R/$2" 2>/dev/null; then echo "$3=$YES"; else echo "$3=$NO"; fi
}
cfind() {
  if find "$R" -maxdepth $1 \( -name "$2" \) 2>/dev/null | grep -q .; then echo "$3=$YES"; else echo "$3=$NO"; fi
}
cfind2() {
  if find "$R" -maxdepth $1 \( -name "$2" -o -name "$3" \) 2>/dev/null | grep -q .; then echo "$4=$YES"; else echo "$4=$NO"; fi
}

# ── Governance files ──────────────────────────────────────────────────────────
cfind2 2 "LICENSE" "LICENSE.md" LICENSE
cfind2 2 "COPYING" "LICENSE.txt" LICENSE2
# Hyphenated variants common in Rust crates: LICENSE-MIT, LICENSE-APACHE, etc.
if find "$R" -maxdepth 2 -name "LICENSE-*" 2>/dev/null | grep -q .; then echo "LICENSE3=$YES"; else echo "LICENSE3=$NO"; fi
cfind2 1 "README.md" "README.rst" README
cfind2 3 "CHANGELOG.md" "CHANGES.md" CHANGELOG
cfind2 3 "HISTORY.md" "CHANGELOG.rst" CHANGELOG2
cfind2 3 "CONTRIBUTING.md" "CONTRIBUTING.rst" CONTRIBUTING
cfind2 3 "SECURITY.md" "SECURITY.txt" SECURITY_ROOT
c ".github/SECURITY.md" SECURITY_GH
c "docs/SECURITY.md" SECURITY_DOCS
cfind2 3 "CODE_OF_CONDUCT.md" "CODE_OF_CONDUCT.rst" CODE_OF_CONDUCT

# ── CI configs ────────────────────────────────────────────────────────────────
cdir ".github/workflows" CI_GITHUB
c ".travis.yml" CI_TRAVIS
c ".circleci/config.yml" CI_CIRCLE
c "Jenkinsfile" CI_JENKINS
c ".gitlab-ci.yml" CI_GITLAB
c "bitbucket-pipelines.yml" CI_BITBUCKET

# ── Dependency update automation ──────────────────────────────────────────────
c ".github/dependabot.yml" DEPENDABOT
c ".github/dependabot.yaml" DEPENDABOT2
cfind2 2 "renovate.json" "renovate.json5" RENOVATE
c ".renovaterc" RENOVATERC

# ── Lock files (cross-language) ───────────────────────────────────────────────
c "Cargo.lock" LOCK_CARGO
c "package-lock.json" LOCK_NPM
c "yarn.lock" LOCK_YARN
c "pnpm-lock.yaml" LOCK_PNPM
c "go.sum" LOCK_GO
c "poetry.lock" LOCK_POETRY
c "uv.lock" LOCK_UV
c "Pipfile.lock" LOCK_PIPFILE
c "composer.lock" LOCK_COMPOSER
c "Gemfile.lock" LOCK_GEM

# ── Lint / static analysis configs ───────────────────────────────────────────
c "clippy.toml" LINT_CLIPPY
c ".clippy.toml" LINT_CLIPPY2
c ".eslintrc.js" LINT_ESLINT
c ".eslintrc.cjs" LINT_ESLINT2
c ".eslintrc.json" LINT_ESLINT3
c "eslint.config.js" LINT_ESLINT4
c "ruff.toml" LINT_RUFF
c ".ruff.toml" LINT_RUFF2
c ".flake8" LINT_FLAKE8
c ".golangci.yml" LINT_GOLANG
c ".golangci.yaml" LINT_GOLANG2
c "pylintrc" LINT_PYLINT
c ".pylintrc" LINT_PYLINT2
c "biome.json" LINT_BIOME
c "oxlintrc.json" LINT_OXC
# CodeQL / SAST workflow
cfind 4 "codeql*.yml" LINT_CODEQL

# ── Pre-commit hooks ──────────────────────────────────────────────────────────
c ".pre-commit-config.yaml" PRECOMMIT
cdir ".husky" HUSKY
c "lefthook.yml" LEFTHOOK
c ".lefthook.yml" LEFTHOOK2

# ── Safety / audit configs ────────────────────────────────────────────────────
c "deny.toml" SAFETY_DENY
c ".cargo/audit.toml" SAFETY_AUDIT
cgrep "forbid(unsafe_code)" "src" SAFETY_UNSAFE_FORBID
cgrep "deny(unsafe_code)" "src" SAFETY_UNSAFE_DENY
c "mypy.ini" SAFETY_MYPY
cgrep "strict = true" "pyproject.toml" SAFETY_MYPY_STRICT
c "tsconfig.json" SAFETY_TS
cgrep '"strict": true' "tsconfig.json" SAFETY_TS_STRICT

# ── Test infrastructure ───────────────────────────────────────────────────────
cdir "tests" TESTS_DIR
cdir "test" TEST_DIR
cdir "spec" SPEC_DIR
cdir "__tests__" TESTS_JS
cfind 5 "*_test.go" TESTS_GO
cfind 5 "*_test.rs" TESTS_RS
cfind 5 "test_*.py" TESTS_PY
cfind 5 "*.test.ts" TESTS_TS
cfind 5 "*.spec.ts" TESTS_SPEC_TS
cfind 5 "*.test.js" TESTS_JS2
c "jest.config.js" TESTS_JEST
c "jest.config.ts" TESTS_JEST2
c "vitest.config.ts" TESTS_VITEST
c "pytest.ini" TESTS_PYTEST
cgrep 'pytest' "pyproject.toml" TESTS_PYTEST2

# ── Language-specific version constraints ─────────────────────────────────────
cgrep 'rust-version' "Cargo.toml" RUST_MSRV
cgrep '"engines"' "package.json" NODE_ENGINE
cgrep 'requires-python' "pyproject.toml" PYTHON_REQUIRES
cgrep '^go [0-9]' "go.mod" GO_VERSION
"#;

async fn run_governance_script(
    docker: &std::sync::Arc<bollard::Docker>,
    container_id: &str,
) -> Result<GovernanceReport, CoreError> {
    let result = exec_in_container(
        docker,
        container_id,
        ExecCommand {
            cmd: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                GOVERNANCE_SCRIPT.to_string(),
            ],
            working_dir: None,
            env: vec![],
            timeout_secs: Some(30u64),
        },
    )
    .await?;

    let kv: HashMap<&str, bool> = result
        .stdout
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once('=')?;
            Some((k.trim(), v.trim() == "YES"))
        })
        .collect();

    let yes = |key: &str| *kv.get(key).unwrap_or(&false);

    let has_license = yes("LICENSE") || yes("LICENSE2") || yes("LICENSE3");
    Ok(GovernanceReport {
        has_license,
        license_all_rights_reserved: !has_license,
        has_readme: yes("README"),
        has_changelog: yes("CHANGELOG") || yes("CHANGELOG2"),
        has_contributing: yes("CONTRIBUTING"),
        has_security_policy: yes("SECURITY_ROOT") || yes("SECURITY_GH") || yes("SECURITY_DOCS"),
        has_code_of_conduct: yes("CODE_OF_CONDUCT"),

        has_ci_config: yes("CI_GITHUB")
            || yes("CI_TRAVIS")
            || yes("CI_CIRCLE")
            || yes("CI_JENKINS")
            || yes("CI_GITLAB")
            || yes("CI_BITBUCKET"),
        has_dependabot: yes("DEPENDABOT") || yes("DEPENDABOT2"),
        has_renovate: yes("RENOVATE") || yes("RENOVATERC"),

        has_any_lock_file: yes("LOCK_CARGO")
            || yes("LOCK_NPM")
            || yes("LOCK_YARN")
            || yes("LOCK_PNPM")
            || yes("LOCK_GO")
            || yes("LOCK_POETRY")
            || yes("LOCK_UV")
            || yes("LOCK_PIPFILE")
            || yes("LOCK_COMPOSER")
            || yes("LOCK_GEM"),

        has_lint_config: yes("LINT_CLIPPY")
            || yes("LINT_CLIPPY2")
            || yes("LINT_ESLINT")
            || yes("LINT_ESLINT2")
            || yes("LINT_ESLINT3")
            || yes("LINT_ESLINT4")
            || yes("LINT_RUFF")
            || yes("LINT_RUFF2")
            || yes("LINT_FLAKE8")
            || yes("LINT_GOLANG")
            || yes("LINT_GOLANG2")
            || yes("LINT_PYLINT")
            || yes("LINT_PYLINT2")
            || yes("LINT_BIOME")
            || yes("LINT_OXC")
            || yes("LINT_CODEQL"),

        has_pre_commit: yes("PRECOMMIT") || yes("HUSKY") || yes("LEFTHOOK") || yes("LEFTHOOK2"),

        has_safety_config: yes("SAFETY_DENY")
            || yes("SAFETY_AUDIT")
            || yes("SAFETY_UNSAFE_FORBID")
            || yes("SAFETY_UNSAFE_DENY")
            || yes("SAFETY_MYPY")
            || yes("SAFETY_MYPY_STRICT")
            || yes("SAFETY_TS_STRICT"),

        has_test_files: yes("TESTS_DIR")
            || yes("TEST_DIR")
            || yes("SPEC_DIR")
            || yes("TESTS_JS")
            || yes("TESTS_GO")
            || yes("TESTS_RS")
            || yes("TESTS_PY")
            || yes("TESTS_TS")
            || yes("TESTS_SPEC_TS")
            || yes("TESTS_JS2")
            || yes("TESTS_JEST")
            || yes("TESTS_JEST2")
            || yes("TESTS_VITEST")
            || yes("TESTS_PYTEST")
            || yes("TESTS_PYTEST2"),

        rust_msrv_declared: yes("RUST_MSRV"),
        node_engine_declared: yes("NODE_ENGINE"),
        python_requires_declared: yes("PYTHON_REQUIRES"),
        go_version_declared: yes("GO_VERSION"),
    })
}

// ── Docker-free local governance check ───────────────────────────────────────

/// Check which governance files are present in a local repository directory.
/// Used by the `--no-container` code path.
///
/// Performs only filename/directory existence checks; does not grep file
/// contents, so sub-fields like `rust_msrv_declared` remain `false`.
pub fn detect_governance_local(path: &std::path::Path) -> GovernanceReport {
    let has = |name: &str| path.join(name).exists();
    let has_any = |names: &[&str]| names.iter().any(|n| has(n));
    let has_dir = |name: &str| path.join(name).is_dir();

    let has_license = has_any(&[
        "LICENSE", "LICENSE.md", "LICENSE.txt", "LICENCE",
        "LICENSE-MIT", "LICENSE-APACHE", "COPYING",
    ]);

    GovernanceReport {
        has_license,
        license_all_rights_reserved: !has_license,
        has_readme: has_any(&["README.md", "README.txt", "README.rst", "README"]),
        has_changelog: has_any(&["CHANGELOG.md", "CHANGELOG.txt", "CHANGES.md", "HISTORY.md"]),
        has_contributing: has_any(&["CONTRIBUTING.md", "CONTRIBUTING.txt", "CONTRIBUTING.rst"]),
        has_security_policy: has_any(&[
            "SECURITY.md", ".github/SECURITY.md", "docs/SECURITY.md", "SECURITY.txt",
        ]),
        has_code_of_conduct: has_any(&["CODE_OF_CONDUCT.md", "CODE_OF_CONDUCT.rst"]),
        has_ci_config: has_dir(".github/workflows")
            || has(".travis.yml")
            || has(".circleci/config.yml")
            || has("Jenkinsfile")
            || has(".gitlab-ci.yml")
            || has("bitbucket-pipelines.yml"),
        has_dependabot: has(".github/dependabot.yml") || has(".github/dependabot.yaml"),
        has_renovate: has("renovate.json") || has("renovate.json5") || has(".renovaterc"),
        has_any_lock_file: has_any(&[
            "Cargo.lock", "package-lock.json", "yarn.lock", "pnpm-lock.yaml",
            "go.sum", "poetry.lock", "uv.lock", "Pipfile.lock", "Gemfile.lock",
            "composer.lock",
        ]),
        has_lint_config: has_any(&[
            "clippy.toml", ".clippy.toml",
            ".eslintrc.js", ".eslintrc.cjs", ".eslintrc.json", "eslint.config.js",
            "ruff.toml", ".ruff.toml", ".flake8",
            ".golangci.yml", ".golangci.yaml",
            "pylintrc", ".pylintrc",
            "biome.json", "oxlintrc.json",
        ]),
        has_pre_commit: has(".pre-commit-config.yaml") || has_dir(".husky") || has("lefthook.yml"),
        has_safety_config: has_any(&["deny.toml", ".cargo/audit.toml", "mypy.ini", "tsconfig.json"]),
        has_test_files: has_dir("tests")
            || has_dir("test")
            || has_dir("spec")
            || has_dir("__tests__"),
        // Content-grep signals are not feasible without reading files — leave false.
        rust_msrv_declared: false,
        node_engine_declared: false,
        python_requires_declared: false,
        go_version_declared: false,
    }
}
