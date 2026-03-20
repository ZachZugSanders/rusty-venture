use std::sync::Arc;

use async_trait::async_trait;
use bollard::Docker;
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::container::{exec_in_container, ExecCommand, CTX_CONTAINER_ID, CTX_DOCKER_CLIENT};
use super::clone::CTX_REPO_LOCAL_PATH;

pub const CTX_DETECTED_LANGUAGES: &str = "repo.detected_languages";

/// A programming language/runtime that can be detected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Rust,
    Node,
    Python,
    Go,
    Java,
    Ruby,
    PHP,
    CSharp,
    Swift,
    Kotlin,
    Unknown,
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Language::Rust => "Rust",
            Language::Node => "Node.js",
            Language::Python => "Python",
            Language::Go => "Go",
            Language::Java => "Java",
            Language::Ruby => "Ruby",
            Language::PHP => "PHP",
            Language::CSharp => "C#",
            Language::Swift => "Swift",
            Language::Kotlin => "Kotlin",
            Language::Unknown => "Unknown",
        };
        write!(f, "{s}")
    }
}

/// Language marker files and their detection weights.
/// Higher weight = stronger signal for that language.
const MARKERS: &[(&str, Language, u8)] = &[
    ("Cargo.toml", Language::Rust, 10),
    ("package.json", Language::Node, 10),
    ("requirements.txt", Language::Python, 8),
    ("pyproject.toml", Language::Python, 9),
    ("setup.py", Language::Python, 8),
    ("setup.cfg", Language::Python, 7),
    ("go.mod", Language::Go, 10),
    ("pom.xml", Language::Java, 10),
    ("build.gradle", Language::Java, 9),
    ("build.gradle.kts", Language::Kotlin, 9),
    ("Gemfile", Language::Ruby, 10),
    ("composer.json", Language::PHP, 10),
    ("Package.swift", Language::Swift, 10),
    (".csproj", Language::CSharp, 9),
    (".sln", Language::CSharp, 8),
];

/// The result of language detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedLanguages {
    pub primary: Language,
    /// Additional languages found in the repo (monorepos, polyglot projects).
    pub secondary: Vec<Language>,
    /// All detected languages with their scores, sorted descending.
    pub scores: Vec<(Language, u8)>,
}

/// Detects the primary programming language by scoring marker files.
#[derive(Default)]
pub struct DetectLanguageAction;

#[async_trait]
impl Action for DetectLanguageAction {
    type Input = ();
    type Output = DetectedLanguages;

    fn name(&self) -> &str {
        "detect-language"
    }

    async fn execute(&self, ctx: &ExecutionContext, _input: ()) -> Result<DetectedLanguages, CoreError> {
        let container_id: String = ctx.require::<String>(CTX_CONTAINER_ID).await?;
        let docker: Arc<Docker> = ctx.require::<Arc<Docker>>(CTX_DOCKER_CLIENT).await?;
        let repo_path: String = ctx.require::<String>(CTX_REPO_LOCAL_PATH).await?;

        // List all files recursively (just names, depth 3 to avoid huge output)
        let result = exec_in_container(
            &docker,
            &container_id,
            ExecCommand::new(["find", &repo_path, "-maxdepth", "3", "-type", "f", "-printf", "%f\n"])
                .working_dir(&repo_path),
        )
        .await?;

        let file_listing = result.stdout;

        // Also list root files for higher-confidence detection
        let root_result = exec_in_container(
            &docker,
            &container_id,
            ExecCommand::new(["ls", "-1", &repo_path])
                .working_dir(&repo_path),
        )
        .await?;

        let root_files = root_result.stdout;

        // Score each language
        use std::collections::HashMap;
        let mut scores: HashMap<&Language, u8> = HashMap::new();

        for (marker, language, weight) in MARKERS {
            // Root-level markers get full weight; nested markers get half weight
            if root_files.lines().any(|f| {
                if marker.starts_with('.') {
                    f.ends_with(marker)
                } else {
                    f == *marker
                }
            }) {
                *scores.entry(language).or_insert(0) += weight;
            } else if file_listing.lines().any(|f| {
                if marker.starts_with('.') {
                    f.ends_with(marker)
                } else {
                    f == *marker
                }
            }) {
                *scores.entry(language).or_insert(0) += weight / 2;
            }
        }

        let mut sorted_scores: Vec<(Language, u8)> = scores
            .into_iter()
            .map(|(lang, score)| (lang.clone(), score))
            .collect();
        sorted_scores.sort_by(|a, b| b.1.cmp(&a.1));

        let primary = sorted_scores
            .first()
            .map(|(l, _)| l.clone())
            .unwrap_or(Language::Unknown);

        let secondary: Vec<Language> = sorted_scores
            .iter()
            .skip(1)
            .filter(|(_, score)| *score > 0)
            .map(|(l, _)| l.clone())
            .collect();

        info!(
            primary = %primary,
            secondary_count = secondary.len(),
            "Language detection complete"
        );

        let detected = DetectedLanguages {
            primary,
            secondary,
            scores: sorted_scores,
        };

        ctx.insert(CTX_DETECTED_LANGUAGES, detected.clone()).await;
        Ok(detected)
    }
}

// ── Docker-free local detection ───────────────────────────────────────────────

/// Detect the primary programming language by checking well-known marker files
/// directly on the local filesystem. Used by the `--no-container` code path.
///
/// Returns the first matching language in priority order. For a full scored
/// multi-language result use the Docker-based `DetectLanguageAction` instead.
pub fn detect_language_local(path: &std::path::Path) -> Language {
    let markers: &[(&str, Language)] = &[
        ("Cargo.toml", Language::Rust),
        ("go.mod", Language::Go),
        ("pyproject.toml", Language::Python),
        ("requirements.txt", Language::Python),
        ("package.json", Language::Node),
        ("pom.xml", Language::Java),
        ("build.gradle", Language::Java),
        ("build.gradle.kts", Language::Kotlin),
        ("Gemfile", Language::Ruby),
        ("composer.json", Language::PHP),
        ("Package.swift", Language::Swift),
    ];
    for (file, lang) in markers {
        if path.join(file).exists() {
            return lang.clone();
        }
    }
    Language::Unknown
}
