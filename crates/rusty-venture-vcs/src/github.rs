use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT},
    Client, StatusCode,
};
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::{
    provider::{CodeSearchHit, CommitSha, FileCommit, RemoteRepo, RepoProvider, RepoRef},
    VcsError,
};

const GITHUB_API: &str = "https://api.github.com";

// ── GithubProvider ────────────────────────────────────────────────────────────

/// VCS provider backed by the GitHub REST API v3.
///
/// Construct with a personal access token (PAT) that has at least:
/// - `repo` scope for private repositories
/// - `public_repo` scope for public repositories only
pub struct GithubProvider {
    client: Client,
    /// Override for GitHub Enterprise: e.g. `"https://github.mycompany.com/api/v3"`.
    base_url: String,
}

impl GithubProvider {
    pub fn new(token: impl Into<String>) -> Result<Self, VcsError> {
        Self::with_base_url(token, GITHUB_API)
    }

    pub fn with_base_url(
        token: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, VcsError> {
        let token = token.into();
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| VcsError::Auth(e.to_string()))?,
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            HeaderName::from_static("x-github-api-version"),
            HeaderValue::from_static("2022-11-28"),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static("rusty-venture/0.1"));

        let client = Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|e| VcsError::Http(e.to_string()))?;

        Ok(Self {
            client,
            base_url: base_url.into(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// GET with automatic 404→None handling.
    async fn get_optional<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
    ) -> Result<Option<T>, VcsError> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| VcsError::Http(e.to_string()))?;

        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let resp = resp
            .error_for_status()
            .map_err(|e| VcsError::Api(format!("GET {url}: {}", e.status().unwrap_or_default())))?;

        Ok(Some(
            resp.json::<T>()
                .await
                .map_err(|e| VcsError::Parse(e.to_string()))?,
        ))
    }
}

// ── GitHub API response shapes ────────────────────────────────────────────────

#[derive(Deserialize)]
struct GhRepo {
    owner: GhOwner,
    name: String,
    full_name: String,
    default_branch: String,
    clone_url: String,
    description: Option<String>,
    language: Option<String>,
}

#[derive(Deserialize)]
struct GhOwner {
    login: String,
}

impl From<GhRepo> for RemoteRepo {
    fn from(r: GhRepo) -> Self {
        RemoteRepo {
            owner: r.owner.login,
            name: r.name,
            full_name: r.full_name,
            default_branch: r.default_branch,
            clone_url: r.clone_url,
            description: r.description,
            language: r.language,
        }
    }
}

#[derive(Deserialize)]
struct GhRef {
    object: GhRefObject,
}

#[derive(Deserialize)]
struct GhRefObject {
    sha: String,
}

#[derive(Deserialize)]
struct GhContents {
    content: Option<String>, // base64-encoded, only present for files ≤ 1 MB
    sha: String,
    encoding: Option<String>,
}

#[derive(Deserialize)]
struct GhSearchResult {
    items: Vec<GhSearchItem>,
}

#[derive(Deserialize)]
struct GhSearchItem {
    repository: GhRepo,
    path: String,
}

// ── RepoProvider impl ─────────────────────────────────────────────────────────

#[async_trait]
impl RepoProvider for GithubProvider {
    fn provider_name(&self) -> &str {
        "github"
    }

    async fn list_org_repos(&self, org: &str) -> Result<Vec<RemoteRepo>, VcsError> {
        let mut repos = Vec::new();
        let mut page = 1u32;

        loop {
            let url = self.url(&format!(
                "/orgs/{org}/repos?per_page=100&page={page}&type=all"
            ));
            debug!(url = %url, "Fetching org repos page {page}");

            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| VcsError::Http(e.to_string()))?
                .error_for_status()
                .map_err(|e| VcsError::Api(format!("list_org_repos: {e}")))?;

            let page_repos: Vec<GhRepo> = resp
                .json()
                .await
                .map_err(|e| VcsError::Parse(e.to_string()))?;

            let count = page_repos.len();
            repos.extend(page_repos.into_iter().map(RemoteRepo::from));

            if count < 100 {
                break;
            }
            page += 1;
        }

        info!(org, count = repos.len(), "Listed org repos");
        Ok(repos)
    }

    async fn search_code(
        &self,
        query: &str,
        filename_glob: &str,
    ) -> Result<Vec<CodeSearchHit>, VcsError> {
        // GitHub code search: q=<query>+filename:<glob>
        let q = format!("{query} filename:{filename_glob}");
        let url = self.url(&format!(
            "/search/code?q={}&per_page=100",
            urlencoding::encode(&q)
        ));

        debug!(query = %q, "Searching code");

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| VcsError::Http(e.to_string()))?;

        // 422 = query validation error (e.g. too short), treat as empty result.
        if resp.status() == StatusCode::UNPROCESSABLE_ENTITY {
            warn!(query = %q, "Code search query rejected by GitHub (422)");
            return Ok(vec![]);
        }

        let result: GhSearchResult = resp
            .error_for_status()
            .map_err(|e| VcsError::Api(format!("search_code: {e}")))?
            .json()
            .await
            .map_err(|e| VcsError::Parse(e.to_string()))?;

        let hits = result
            .items
            .into_iter()
            .map(|item| CodeSearchHit {
                repo: RemoteRepo::from(item.repository),
                file_path: item.path,
            })
            .collect::<Vec<_>>();

        info!(query = %q, count = hits.len(), "Code search complete");
        Ok(hits)
    }

    async fn create_branch(
        &self,
        repo: &RepoRef,
        branch: &str,
        from_ref: &str,
    ) -> Result<(), VcsError> {
        // Resolve from_ref to a SHA (handles branch names).
        let ref_url = self.url(&format!(
            "/repos/{}/git/ref/heads/{}",
            repo.full_name(),
            urlencoding::encode(from_ref)
        ));

        let gh_ref: Option<GhRef> = self.get_optional(&ref_url).await?;
        let sha = match gh_ref {
            Some(r) => r.object.sha,
            None => {
                // from_ref might already be a SHA
                from_ref.to_string()
            }
        };

        let url = self.url(&format!("/repos/{}/git/refs", repo.full_name()));
        let body = serde_json::json!({
            "ref": format!("refs/heads/{branch}"),
            "sha": sha,
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| VcsError::Http(e.to_string()))?;

        // 422 = branch already exists; treat as idempotent success.
        if resp.status() == StatusCode::UNPROCESSABLE_ENTITY {
            info!(repo = %repo, branch, "Branch already exists, skipping create");
            return Ok(());
        }

        resp.error_for_status()
            .map_err(|e| VcsError::Api(format!("create_branch: {e}")))?;

        info!(repo = %repo, branch, from_ref, "Branch created");
        Ok(())
    }

    async fn get_file(
        &self,
        repo: &RepoRef,
        path: &str,
        git_ref: &str,
    ) -> Result<Option<(String, String)>, VcsError> {
        let url = self.url(&format!(
            "/repos/{}/contents/{}?ref={}",
            repo.full_name(),
            urlencoding::encode(path),
            urlencoding::encode(git_ref)
        ));

        let contents: Option<GhContents> = self.get_optional(&url).await?;

        match contents {
            None => Ok(None),
            Some(c) => {
                let encoding = c.encoding.as_deref().unwrap_or("base64");
                if encoding != "base64" {
                    return Err(VcsError::Parse(format!(
                        "Unsupported file encoding '{encoding}' for {path}"
                    )));
                }
                let raw = c.content.ok_or_else(|| {
                    VcsError::Parse(format!(
                        "File {path} is too large to retrieve via contents API"
                    ))
                })?;
                // GitHub includes newlines in the base64 payload — strip them.
                let cleaned: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
                let bytes = B64
                    .decode(cleaned)
                    .map_err(|e| VcsError::Parse(format!("base64 decode {path}: {e}")))?;
                let text = String::from_utf8(bytes)
                    .map_err(|e| VcsError::Parse(format!("UTF-8 decode {path}: {e}")))?;
                Ok(Some((text, c.sha)))
            }
        }
    }

    async fn commit_files(
        &self,
        repo: &RepoRef,
        branch: &str,
        files: Vec<FileCommit>,
        message: &str,
    ) -> Result<CommitSha, VcsError> {
        // GitHub's Contents API supports one file per request.
        // We commit them sequentially; for an atomic multi-file commit the
        // Git Data API (tree+commit) would be needed — that's a future enhancement.
        let mut last_sha = String::new();

        for file in files {
            let url = self.url(&format!(
                "/repos/{}/contents/{}",
                repo.full_name(),
                urlencoding::encode(&file.path)
            ));

            let content_b64 = B64.encode(file.content.as_bytes());

            let mut body = serde_json::json!({
                "message": message,
                "content": content_b64,
                "branch":  branch,
            });

            if let Some(sha) = file.existing_sha {
                body["sha"] = serde_json::Value::String(sha);
            }

            #[derive(Deserialize)]
            struct CommitResp {
                commit: CommitObj,
            }
            #[derive(Deserialize)]
            struct CommitObj {
                sha: String,
            }

            let resp: CommitResp = self
                .client
                .put(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| VcsError::Http(e.to_string()))?
                .error_for_status()
                .map_err(|e| VcsError::Api(format!("commit_files {}: {e}", file.path)))?
                .json()
                .await
                .map_err(|e| VcsError::Parse(e.to_string()))?;

            last_sha = resp.commit.sha;
            info!(repo = %repo, path = %file.path, sha = %last_sha, "File committed");
        }

        Ok(CommitSha(last_sha))
    }
}
