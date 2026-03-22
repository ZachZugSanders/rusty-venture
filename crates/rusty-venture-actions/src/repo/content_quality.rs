/// Tier 2 — Content Quality signals.
///
/// These checks read and analyse *file contents* rather than simply checking
/// file presence (Tier 1). They run only when `scan_tier >= 2` and rely on
/// a POSIX shell script executed inside the analysis container.
use std::collections::HashMap;

use async_trait::async_trait;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::container::{exec_in_container, ExecCommand, CTX_CONTAINER_ID, CTX_DOCKER_CLIENT};

pub const CTX_CONTENT_QUALITY_REPORT: &str = "content_quality.report";

/// Results from Tier 2 content-inspection checks.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContentQualityReport {
    // ── README substance ──────────────────────────────────────────────────
    /// Approximate word count of the README file (0 if absent).
    pub readme_word_count: u32,
    /// README contains at least one Markdown/RST section heading.
    pub readme_has_headings: bool,
    /// README contains at least one fenced or indented code block.
    pub readme_has_code_blocks: bool,

    // ── Changelog quality ─────────────────────────────────────────────────
    /// CHANGELOG contains versioned entries matching a semver pattern.
    pub changelog_has_versions: bool,

    // ── License SPDX ─────────────────────────────────────────────────────
    /// LICENSE file contains a recognised SPDX identifier keyword.
    pub license_is_spdx: bool,

    // ── CI test commands ──────────────────────────────────────────────────
    /// CI configuration file(s) reference an actual test command.
    pub ci_has_test_command: bool,

    // ── Dockerfile non-root user ──────────────────────────────────────────
    /// At least one Dockerfile has a `USER` directive for a non-root user.
    pub dockerfile_has_nonroot_user: bool,

    // ── Test function density ─────────────────────────────────────────────
    /// Number of test function definitions found across all test files.
    pub test_function_count: u32,

    // ── Lock file freshness ───────────────────────────────────────────────
    /// Lock file is NOT older than the dependency manifest (i.e. not stale).
    pub lockfile_not_stale: bool,
    /// Whether a lock+manifest pair was found at all (used to skip the signal).
    pub lockfile_pair_found: bool,

    /// `true` when the script was actually executed (scan_tier >= 2 in container mode).
    pub was_checked: bool,
}

/// Runs content-inspection shell checks in the analysis container (Tier 2).
///
/// This action is a no-op when the execution context does not contain a
/// `CTX_CONTAINER_ID` (i.e. the no-container path), in which case it returns
/// a default empty report.
pub struct ContentQualityCheckAction;

#[async_trait]
impl Action for ContentQualityCheckAction {
    type Input = ();
    type Output = ContentQualityReport;

    fn name(&self) -> &str {
        "content-quality-check"
    }

    async fn execute(
        &self,
        ctx: &ExecutionContext,
        _input: (),
    ) -> Result<ContentQualityReport, CoreError> {
        use bollard::Docker;
        use std::sync::Arc;

        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;

        let report = run_content_quality_script(&docker, &container_id).await?;

        info!(
            readme_words = report.readme_word_count,
            ci_has_test_cmd = report.ci_has_test_command,
            test_fn_count = report.test_function_count,
            license_spdx = report.license_is_spdx,
            lockfile_fresh = report.lockfile_not_stale,
            "Content quality check complete"
        );

        ctx.insert(CTX_CONTENT_QUALITY_REPORT, report.clone()).await;
        Ok(report)
    }
}

// ── Internal shell script ─────────────────────────────────────────────────────

/// POSIX sh script that outputs `KEY=VALUE` lines.
/// Uses only coreutils and find/grep/wc available in the analysis runner image.
const CONTENT_QUALITY_SCRIPT: &str = r#"
R=/workspace/repo
YES=YES
NO=NO

# ── README substance ─────────────────────────────────────────────────────────
README_FILE=""
for f in README.md README.rst README.txt README; do
  [ -f "$R/$f" ] && README_FILE="$R/$f" && break
done

if [ -n "$README_FILE" ]; then
  README_WORDS=$(wc -w < "$README_FILE" 2>/dev/null || echo 0)
  README_HEADINGS=$(grep -cE '^#{1,6} |^[A-Za-z].+$' "$README_FILE" 2>/dev/null | head -1 || echo 0)
  README_HEADINGS_ACTUAL=$(grep -cE '^#{1,6} ' "$README_FILE" 2>/dev/null || echo 0)
  README_CODE=$(grep -c '^\`\`\`' "$README_FILE" 2>/dev/null || echo 0)
else
  README_WORDS=0
  README_HEADINGS_ACTUAL=0
  README_CODE=0
fi

echo "README_WORDS=${README_WORDS:-0}"
[ "${README_HEADINGS_ACTUAL:-0}" -gt 0 ] && echo "README_HEADINGS=$YES" || echo "README_HEADINGS=$NO"
[ "${README_CODE:-0}" -gt 0 ] && echo "README_CODE=$YES" || echo "README_CODE=$NO"

# ── CHANGELOG semver entries ─────────────────────────────────────────────────
CHANGELOG_FILE=""
for f in CHANGELOG.md CHANGES.md HISTORY.md CHANGELOG.rst CHANGELOG; do
  [ -f "$R/$f" ] && CHANGELOG_FILE="$R/$f" && break
done

if [ -n "$CHANGELOG_FILE" ] && grep -qE '##\s+\[?v?[0-9]+\.[0-9]+|^[0-9]+\.[0-9]+\.[0-9]+' "$CHANGELOG_FILE" 2>/dev/null; then
  echo "CHANGELOG_VERSIONS=$YES"
else
  echo "CHANGELOG_VERSIONS=$NO"
fi

# ── License SPDX keyword ─────────────────────────────────────────────────────
LICENSE_FILE=""
for f in LICENSE LICENSE.md LICENSE.txt LICENCE LICENSE-MIT LICENSE-APACHE COPYING; do
  [ -f "$R/$f" ] && LICENSE_FILE="$R/$f" && break
done

if [ -n "$LICENSE_FILE" ] && grep -qiE 'MIT License|Apache License|BSD [0-9]|GNU General Public|GNU Lesser|Mozilla Public|ISC License|CC0|The Unlicense|European Union Public|Eclipse Public|Creative Commons' "$LICENSE_FILE" 2>/dev/null; then
  echo "LICENSE_SPDX=$YES"
else
  echo "LICENSE_SPDX=$NO"
fi

# ── CI references a test command ─────────────────────────────────────────────
CI_TEST=$NO

# GitHub Actions workflows
if [ -d "$R/.github/workflows" ]; then
  if find "$R/.github/workflows" -name '*.yml' -o -name '*.yaml' 2>/dev/null | \
     xargs grep -qlE 'cargo test|cargo check|npm test|yarn test|bun test|pytest|go test|mvn test|gradle test|bundle exec rspec|dotnet test|vitest|jest' 2>/dev/null; then
    CI_TEST=$YES
  fi
fi

# Other CI providers
for ci_file in "$R/.travis.yml" "$R/.circleci/config.yml" "$R/.gitlab-ci.yml" "$R/bitbucket-pipelines.yml"; do
  if [ -f "$ci_file" ] && grep -qiE 'test|check|spec|verify' "$ci_file" 2>/dev/null; then
    CI_TEST=$YES
    break
  fi
done

echo "CI_TEST=$CI_TEST"

# ── Dockerfile non-root USER directive ───────────────────────────────────────
DOCKER_NONROOT=$NO
for f in $(find "$R" -maxdepth 4 -name 'Dockerfile' -o -name 'Dockerfile.*' 2>/dev/null | head -10); do
  if [ -f "$f" ]; then
    # Check for USER directive that is NOT root or 0
    USER_LINES=$(grep -E '^USER ' "$f" 2>/dev/null || true)
    if [ -n "$USER_LINES" ]; then
      # If at least one USER line doesn't resolve to root/0
      NON_ROOT=$(echo "$USER_LINES" | grep -vE '^USER\s+(root|0)\s*$' || true)
      if [ -n "$NON_ROOT" ]; then
        DOCKER_NONROOT=$YES
        break
      fi
    fi
  fi
done
echo "DOCKER_NONROOT=$DOCKER_NONROOT"

# ── Test function count ───────────────────────────────────────────────────────
T=0

# Rust: #[test] attribute
RUST_TESTS=$(grep -r '#\[test\]' "$R" --include='*.rs' 2>/dev/null | wc -l || echo 0)
T=$(( T + RUST_TESTS ))

# Python: def test_*
PY_TESTS=$(grep -r 'def test_' "$R" --include='*.py' 2>/dev/null | wc -l || echo 0)
T=$(( T + PY_TESTS ))

# Go: func Test
GO_TESTS=$(grep -r 'func Test' "$R" --include='*.go' 2>/dev/null | wc -l || echo 0)
T=$(( T + GO_TESTS ))

# JS/TS test files: it( or test( at line start / after indentation
JS_TESTS=$(find "$R" \( -name '*.test.ts' -o -name '*.test.js' -o -name '*.spec.ts' -o -name '*.spec.js' \) 2>/dev/null | \
  xargs grep -hE "^\s*(it|test)\(" 2>/dev/null | wc -l || echo 0)
T=$(( T + JS_TESTS ))

# Java/Kotlin: @Test annotation
JVM_TESTS=$(grep -r '@Test' "$R" --include='*.java' --include='*.kt' 2>/dev/null | wc -l || echo 0)
T=$(( T + JVM_TESTS ))

echo "TEST_FN_COUNT=$T"

# ── Lock file freshness ───────────────────────────────────────────────────────
LOCK_FILE=""
MANIFEST_FILE=""

# Priority: use the first pair found
[ -f "$R/Cargo.lock" ]          && [ -f "$R/Cargo.toml" ]      && LOCK_FILE="$R/Cargo.lock"          && MANIFEST_FILE="$R/Cargo.toml"
[ -z "$LOCK_FILE" ] && [ -f "$R/package-lock.json" ] && [ -f "$R/package.json" ] && LOCK_FILE="$R/package-lock.json" && MANIFEST_FILE="$R/package.json"
[ -z "$LOCK_FILE" ] && [ -f "$R/yarn.lock" ]         && [ -f "$R/package.json" ] && LOCK_FILE="$R/yarn.lock"         && MANIFEST_FILE="$R/package.json"
[ -z "$LOCK_FILE" ] && [ -f "$R/poetry.lock" ]       && [ -f "$R/pyproject.toml" ] && LOCK_FILE="$R/poetry.lock"       && MANIFEST_FILE="$R/pyproject.toml"
[ -z "$LOCK_FILE" ] && [ -f "$R/go.sum" ]            && [ -f "$R/go.mod" ]        && LOCK_FILE="$R/go.sum"            && MANIFEST_FILE="$R/go.mod"

if [ -n "$LOCK_FILE" ] && [ -n "$MANIFEST_FILE" ]; then
  echo "LOCKFILE_PAIR=$YES"
  # Stale = manifest is NEWER than lock file
  if [ "$MANIFEST_FILE" -nt "$LOCK_FILE" ]; then
    echo "LOCKFILE_STALE=$YES"
  else
    echo "LOCKFILE_STALE=$NO"
  fi
else
  echo "LOCKFILE_PAIR=$NO"
  echo "LOCKFILE_STALE=$NO"
fi
"#;

async fn run_content_quality_script(
    docker: &std::sync::Arc<bollard::Docker>,
    container_id: &str,
) -> Result<ContentQualityReport, CoreError> {
    let result = exec_in_container(
        docker,
        container_id,
        ExecCommand {
            cmd: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                CONTENT_QUALITY_SCRIPT.to_string(),
            ],
            working_dir: None,
            env: vec![],
            timeout_secs: Some(60u64),
        },
    )
    .await?;

    let kv: HashMap<&str, &str> = result
        .stdout
        .lines()
        .filter_map(|line| {
            let (k, v) = line.split_once('=')?;
            Some((k.trim(), v.trim()))
        })
        .collect();

    let yes = |key: &str| kv.get(key).copied().unwrap_or("NO") == "YES";
    let num = |key: &str| -> u32 { kv.get(key).and_then(|v| v.parse().ok()).unwrap_or(0) };

    Ok(ContentQualityReport {
        readme_word_count: num("README_WORDS"),
        readme_has_headings: yes("README_HEADINGS"),
        readme_has_code_blocks: yes("README_CODE"),
        changelog_has_versions: yes("CHANGELOG_VERSIONS"),
        license_is_spdx: yes("LICENSE_SPDX"),
        ci_has_test_command: yes("CI_TEST"),
        dockerfile_has_nonroot_user: yes("DOCKER_NONROOT"),
        test_function_count: num("TEST_FN_COUNT"),
        lockfile_not_stale: !yes("LOCKFILE_STALE"),
        lockfile_pair_found: yes("LOCKFILE_PAIR"),
        was_checked: true,
    })
}

// ── Docker-free local content quality check ───────────────────────────────────

/// Perform content-quality checks against a local repository directory.
/// Used by the `--no-container` code path.
pub fn detect_content_quality_local(path: &std::path::Path) -> ContentQualityReport {
    // README substance
    let readme_path = ["README.md", "README.rst", "README.txt", "README"]
        .iter()
        .map(|f| path.join(f))
        .find(|p| p.exists());

    let (readme_word_count, readme_has_headings, readme_has_code_blocks) =
        if let Some(p) = readme_path {
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            let words = text.split_whitespace().count() as u32;
            let headings = text.lines().any(|l| l.starts_with('#'));
            let code = text.contains("```");
            (words, headings, code)
        } else {
            (0, false, false)
        };

    // Changelog semver
    let changelog_has_versions = ["CHANGELOG.md", "CHANGES.md", "HISTORY.md", "CHANGELOG"]
        .iter()
        .map(|f| path.join(f))
        .find(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| {
            text.lines().any(|l| {
                let l = l.trim();
                (l.starts_with("## ") || l.starts_with("# "))
                    && l.chars().any(|c| c.is_ascii_digit())
            })
        })
        .unwrap_or(false);

    // License SPDX
    let license_is_spdx = ["LICENSE", "LICENSE.md", "LICENSE.txt", "LICENCE", "COPYING"]
        .iter()
        .map(|f| path.join(f))
        .find(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| {
            let u = text.to_uppercase();
            u.contains("MIT LICENSE")
                || u.contains("APACHE LICENSE")
                || u.contains("BSD ")
                || u.contains("GNU GENERAL PUBLIC")
                || u.contains("GNU LESSER")
                || u.contains("MOZILLA PUBLIC")
                || u.contains("ISC LICENSE")
                || u.contains("CC0")
                || u.contains("THE UNLICENSE")
        })
        .unwrap_or(false);

    // CI test command (basic local check)
    let ci_has_test_command = path.join(".github/workflows").exists()
        && std::fs::read_dir(path.join(".github/workflows"))
            .ok()
            .map(|entries| {
                entries.filter_map(|e| e.ok()).any(|e| {
                    std::fs::read_to_string(e.path())
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains("test")
                })
            })
            .unwrap_or(false);

    // Test function count (simple grep-equivalent)
    let test_function_count = count_test_functions_local(path);

    ContentQualityReport {
        readme_word_count,
        readme_has_headings,
        readme_has_code_blocks,
        changelog_has_versions,
        license_is_spdx,
        ci_has_test_command,
        dockerfile_has_nonroot_user: false, // skip for local path
        test_function_count,
        lockfile_not_stale: true, // can't reliably compare mtimes from a fresh clone
        lockfile_pair_found: false,
        was_checked: true,
    }
}

fn count_test_functions_local(path: &std::path::Path) -> u32 {
    let mut count = 0u32;
    for entry in walkdir_simple(path) {
        let text = std::fs::read_to_string(&entry).unwrap_or_default();
        let ext = entry.extension().and_then(|e| e.to_str()).unwrap_or("");
        count += match ext {
            "rs" => text.lines().filter(|l| l.contains("#[test]")).count() as u32,
            "py" => text
                .lines()
                .filter(|l| l.trim_start().starts_with("def test_"))
                .count() as u32,
            "go" => text.lines().filter(|l| l.starts_with("func Test")).count() as u32,
            _ => 0,
        };
    }
    count
}

/// Minimal recursive directory walker returning file paths (no external deps).
fn walkdir_simple(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.is_dir() {
                // Skip .git and node_modules
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name != ".git" && name != "node_modules" && name != "target" {
                    result.extend(walkdir_simple(&p));
                }
            } else {
                result.push(p);
            }
        }
    }
    result
}
