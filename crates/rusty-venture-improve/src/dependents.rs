use std::sync::Arc;

use async_trait::async_trait;
use rusty_venture_actions::repo::{
    analyze_deps::DependencyReport,
    detect_language::{DetectedLanguages, Language},
    CTX_DEPENDENCY_REPORT, CTX_DETECTED_LANGUAGES, CTX_REPO_URL,
};
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use rusty_venture_vcs::{CodeSearchHit, RemoteRepo, RepoProvider};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

pub const CTX_DEPENDENT_REPOS: &str = "improve.dependent_repos";

// ── Result type ───────────────────────────────────────────────────────────────

/// All repositories discovered as dependents of the analysed repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependentRepos {
    /// The package name that was searched for.
    pub package_name: String,
    /// Repositories that import/depend on this package.
    pub repos: Vec<RemoteRepo>,
    /// Raw code-search hits (includes file paths for context).
    pub hits: Vec<DependentHit>,
}

/// A single search hit indicating dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependentHit {
    pub repo_full_name: String,
    pub file_path: String,
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Discovers repositories that depend on the analysed package by searching
/// for its name inside manifest files via the configured VCS provider.
///
/// Searches three sources in parallel:
/// - Same-org repositories via `list_org_repos`
/// - Code search across the provider for manifest file references
/// - (Optionally) public registries — left as a future enhancement
pub struct DiscoverDependentsAction {
    provider: Arc<dyn RepoProvider>,
    /// If set, restrict org repo scan to this org. Inferred from repo URL if None.
    org_override: Option<String>,
}

impl DiscoverDependentsAction {
    pub fn new(provider: Arc<dyn RepoProvider>) -> Self {
        Self {
            provider,
            org_override: None,
        }
    }

    pub fn org(mut self, org: impl Into<String>) -> Self {
        self.org_override = Some(org.into());
        self
    }
}

#[async_trait]
impl Action for DiscoverDependentsAction {
    type Input = ();
    type Output = DependentRepos;

    fn name(&self) -> &str {
        "discover-dependents"
    }

    async fn execute(
        &self,
        ctx: &ExecutionContext,
        _input: (),
    ) -> Result<DependentRepos, CoreError> {
        let repo_url = ctx.require::<String>(CTX_REPO_URL).await?;
        let languages: DetectedLanguages = ctx
            .get::<DetectedLanguages>(CTX_DETECTED_LANGUAGES)
            .await
            .unwrap_or_default();
        let deps: DependencyReport = ctx
            .get::<DependencyReport>(CTX_DEPENDENCY_REPORT)
            .await
            .unwrap_or_default();

        // Derive the package name from the manifest / repo URL.
        let package_name = derive_package_name(&repo_url, &deps);
        let manifest_filename = manifest_filename_for(&languages.primary);

        info!(
            package = %package_name,
            manifest = manifest_filename,
            provider = self.provider.provider_name(),
            "Searching for dependent repos"
        );

        // ── Code search: find manifest files that reference our package ───
        let hits_result = self
            .provider
            .search_code(&package_name, manifest_filename)
            .await;

        let (hits, repos): (Vec<DependentHit>, Vec<RemoteRepo>) = match hits_result {
            Ok(raw_hits) => {
                let (hits, repos): (Vec<_>, Vec<_>) = raw_hits
                    .into_iter()
                    .map(|h: CodeSearchHit| {
                        let hit = DependentHit {
                            repo_full_name: h.repo.full_name.clone(),
                            file_path: h.file_path.clone(),
                        };
                        (hit, h.repo)
                    })
                    .unzip();
                (hits, repos)
            }
            Err(e) => {
                warn!(error = %e, "Code search failed — returning empty dependent list");
                (vec![], vec![])
            }
        };

        // ── Org scan: additionally list all repos in the same org ─────────
        let org = self
            .org_override
            .clone()
            .unwrap_or_else(|| infer_org(&repo_url));
        let mut org_repos = if !org.is_empty() {
            match self.provider.list_org_repos(&org).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(org = %org, error = %e, "Org repo listing failed");
                    vec![]
                }
            }
        } else {
            vec![]
        };

        // Deduplicate: remove org repos already found in code search.
        let known_names: std::collections::HashSet<String> =
            repos.iter().map(|r| r.full_name.clone()).collect();
        org_repos.retain(|r| !known_names.contains(&r.full_name));

        // Merge: code-search dependents + same-org repos (which may also depend).
        let mut all_repos = repos;
        all_repos.extend(org_repos);

        info!(
            package = %package_name,
            count = all_repos.len(),
            "Dependent repo discovery complete"
        );

        let result = DependentRepos {
            package_name,
            repos: all_repos,
            hits,
        };

        ctx.insert(CTX_DEPENDENT_REPOS, result.clone()).await;
        Ok(result)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extract a package name from the dependency report or repo URL.
fn derive_package_name(repo_url: &str, deps: &DependencyReport) -> String {
    // The manifest file often contains the package name.
    // For now, derive from the last path component of the repo URL.
    if !deps.manifest_file.is_empty() {
        // Could parse manifest for the name, but that requires language-specific
        // parsing. Use the repo name as a reliable fallback.
    }
    repo_url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit('/')
        .next()
        .unwrap_or(repo_url)
        .to_string()
}

/// Returns the manifest filename to search within for each language.
fn manifest_filename_for(lang: &Language) -> &'static str {
    match lang {
        Language::Rust => "Cargo.toml",
        Language::Node => "package.json",
        Language::Python => "requirements.txt",
        Language::Go => "go.mod",
        Language::Java => "pom.xml",
        Language::Ruby => "Gemfile",
        Language::PHP => "composer.json",
        _ => "Cargo.toml", // safe default
    }
}

/// Extract the organisation/user from a GitHub/GitLab/Azure URL.
fn infer_org(repo_url: &str) -> String {
    // e.g. https://github.com/rust-lang/regex → "rust-lang"
    let url = repo_url.trim_end_matches('/').trim_end_matches(".git");
    let parts: Vec<&str> = url.rsplitn(3, '/').collect();
    // parts[0] = "regex", parts[1] = "rust-lang", parts[2] = "https://github.com"
    if parts.len() >= 2 {
        parts[1].to_string()
    } else {
        String::new()
    }
}
