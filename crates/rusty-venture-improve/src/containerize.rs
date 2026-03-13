use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use rusty_venture_actions::repo::{
    analyze_deps::DependencyReport,
    detect_language::{DetectedLanguages, Language},
    scaffold::{ScaffoldSpec, CTX_SCAFFOLD_SPEC},
    CTX_DEPENDENCY_REPORT, CTX_DETECTED_LANGUAGES, CTX_REPO_URL,
};
use rusty_venture_core::{action::Action, context::ExecutionContext, CoreError};
use rusty_venture_llm::{LlmConnector, LlmRequestBuilder};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

pub const CTX_CONTAINERIZE_RESULT: &str = "improve.containerize_result";

// ── Result type ───────────────────────────────────────────────────────────────

/// Outcome of the containerization action, stored in context for the
/// `CreateBranchAction` to commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerizeResult {
    /// Repo-relative path of the generated Dockerfile (e.g. `"Dockerfile"`).
    pub dockerfile_path: String,
    /// Final validated Dockerfile content.
    pub dockerfile_content: String,
    /// Generated docker-compose.yml content, if the repo looks like a service.
    pub compose_content: Option<String>,
    /// Whether the `docker build` validation passed.
    pub build_validated: bool,
    /// How many LLM fix cycles were needed (0 = first attempt succeeded).
    pub fix_iterations: u32,
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Generates a Dockerfile (and docker-compose.yml when appropriate), validates
/// it with `docker build`, and iteratively asks the LLM to fix any errors until
/// the build succeeds or the retry limit is reached.
///
/// Stores a `ContainerizeResult` in context; the actual branch commit is
/// handled separately by `CreateBranchAction`.
pub struct ContainerizeAction<C> {
    connector: Arc<C>,
    /// Maximum LLM-fix iterations after the first build attempt.
    max_fix_iterations: u32,
}

impl<C: LlmConnector + 'static> ContainerizeAction<C> {
    pub fn new(connector: Arc<C>) -> Self {
        Self {
            connector,
            max_fix_iterations: 5,
        }
    }

    pub fn max_fix_iterations(mut self, n: u32) -> Self {
        self.max_fix_iterations = n;
        self
    }
}

#[async_trait]
impl<C: LlmConnector + 'static> Action for ContainerizeAction<C> {
    type Input = ();
    type Output = ContainerizeResult;

    fn name(&self) -> &str {
        "containerize"
    }

    async fn execute(
        &self,
        ctx: &ExecutionContext,
        _input: (),
    ) -> Result<ContainerizeResult, CoreError> {
        let repo_url = ctx.require::<String>(CTX_REPO_URL).await?;
        let languages: DetectedLanguages =
            ctx.get::<DetectedLanguages>(CTX_DETECTED_LANGUAGES).await.unwrap_or_default();
        let deps: DependencyReport =
            ctx.get::<DependencyReport>(CTX_DEPENDENCY_REPORT).await.unwrap_or_default();
        let scaffold: Option<ScaffoldSpec> = ctx.get::<ScaffoldSpec>(CTX_SCAFFOLD_SPEC).await;

        info!(repo = %repo_url, language = %languages.primary, "Starting containerization");

        // ── 1. Clone repo into a temp directory ───────────────────────────
        let temp_dir = tempfile::tempdir()
            .map_err(|e| CoreError::other(format!("Failed to create temp dir: {e}")))?;

        shallow_clone(&repo_url, temp_dir.path()).await?;
        info!(path = %temp_dir.path().display(), "Repo cloned for containerization");

        // ── 2. Check what already exists ──────────────────────────────────
        let has_dockerfile = temp_dir.path().join("Dockerfile").exists();
        let has_compose = temp_dir.path().join("docker-compose.yml").exists()
            || temp_dir.path().join("compose.yml").exists();

        if has_dockerfile {
            info!("Dockerfile already exists — will improve rather than generate from scratch");
        }

        let existing_dockerfile = if has_dockerfile {
            tokio::fs::read_to_string(temp_dir.path().join("Dockerfile")).await.ok()
        } else {
            None
        };

        // ── 3. Generate initial Dockerfile via LLM ────────────────────────
        let dockerfile_path = "Dockerfile".to_string();
        let mut dockerfile_content = generate_dockerfile(
            &self.connector,
            &languages.primary,
            &deps,
            scaffold.as_ref(),
            existing_dockerfile.as_deref(),
            None, // no error context yet
        )
        .await?;

        // ── 4. Generate docker-compose.yml if appropriate ─────────────────
        let compose_content = if !has_compose && looks_like_service(&languages.primary, &deps) {
            Some(
                generate_compose(
                    &self.connector,
                    &languages.primary,
                    &dockerfile_content,
                    &deps,
                )
                .await?,
            )
        } else if has_compose {
            None // don't overwrite an existing compose file
        } else {
            None
        };

        // ── 5. Agentic build-validate-fix loop ────────────────────────────
        let mut fix_iterations = 0u32;
        let mut build_validated = false;

        for attempt in 1..=(self.max_fix_iterations + 1) {
            // Write the current Dockerfile to the temp clone.
            tokio::fs::write(temp_dir.path().join("Dockerfile"), &dockerfile_content)
                .await
                .map_err(|e| CoreError::other(format!("Failed to write Dockerfile: {e}")))?;

            let image_tag = format!(
                "rusty-venture-validate-{}:{}",
                ctx.run_id, attempt
            );

            info!(attempt, tag = %image_tag, "Running docker build");

            match docker_build(temp_dir.path(), &image_tag).await {
                Ok(()) => {
                    info!(attempt, "Docker build succeeded");
                    build_validated = true;

                    // Clean up the test image (best-effort).
                    let _ = docker_rmi(&image_tag).await;
                    break;
                }
                Err(build_error) => {
                    warn!(attempt, error = %build_error, "Docker build failed");

                    if attempt > self.max_fix_iterations {
                        warn!(
                            "Max fix iterations ({}) reached — committing best-effort Dockerfile",
                            self.max_fix_iterations
                        );
                        break;
                    }

                    // Ask LLM to diagnose and fix.
                    info!(attempt, "Asking LLM to diagnose build failure");
                    dockerfile_content = diagnose_and_fix(
                        &self.connector,
                        &languages.primary,
                        &dockerfile_content,
                        &build_error,
                    )
                    .await?;
                    fix_iterations += 1;
                }
            }
        }

        // ── 6. Store result in context ────────────────────────────────────
        let result = ContainerizeResult {
            dockerfile_path,
            dockerfile_content,
            compose_content,
            build_validated,
            fix_iterations,
        };

        info!(
            validated = build_validated,
            fix_iterations,
            "Containerization complete"
        );

        ctx.insert(CTX_CONTAINERIZE_RESULT, result.clone()).await;
        Ok(result)
    }
}

// ── LLM helpers ───────────────────────────────────────────────────────────────

async fn generate_dockerfile<C: LlmConnector>(
    connector: &Arc<C>,
    language: &Language,
    deps: &DependencyReport,
    scaffold: Option<&ScaffoldSpec>,
    existing: Option<&str>,
    error_context: Option<&str>,
) -> Result<String, CoreError> {
    let lang_hints = language_dockerfile_hints(language);
    let manifest = if deps.manifest_file.is_empty() {
        "not detected".to_string()
    } else {
        format!("`{}`", deps.manifest_file)
    };

    let existing_section = existing
        .map(|e| format!("\n## Existing Dockerfile (improve this):\n```dockerfile\n{e}\n```"))
        .unwrap_or_default();

    let error_section = error_context
        .map(|e| {
            format!(
                "\n## Previous Build Error (fix this):\n```\n{}\n```",
                &e[..e.len().min(3000)]
            )
        })
        .unwrap_or_default();

    let scaffold_hints = scaffold
        .map(|s| {
            let file_hints: Vec<String> = s
                .files
                .iter()
                .filter(|f| matches!(f.purpose, rusty_venture_actions::repo::scaffold::FilePurpose::Dockerfile))
                .filter_map(|f| f.template_hint.clone())
                .collect();
            if file_hints.is_empty() {
                String::new()
            } else {
                format!("\n## Scaffold hints:\n{}", file_hints.join("\n"))
            }
        })
        .unwrap_or_default();

    let prompt = format!(
        r#"Generate a production-quality Dockerfile for a {language} application.

## Language: {language}
## Manifest file: {manifest}
## Language-specific guidelines:
{lang_hints}{existing_section}{error_section}{scaffold_hints}

Requirements:
- Use multi-stage builds where applicable to minimise the final image size
- Pin base image versions (not `latest`)
- Run as a non-root user in the final stage
- Set WORKDIR, COPY, and CMD/ENTRYPOINT appropriately
- Include a HEALTHCHECK if the app exposes an HTTP port
- Add a .dockerignore if common build artefacts should be excluded (list them as a comment)

Return ONLY the Dockerfile content — no markdown fences, no explanation."#,
    );

    let request = LlmRequestBuilder::new()
        .system("You are a DevOps expert specialising in Docker containerisation. Return only the requested file content with no surrounding explanation or markdown.")
        .user(prompt)
        .max_tokens(2048)
        .build();

    let response = connector
        .complete(request)
        .await
        .map_err(|e| CoreError::other(format!("LLM Dockerfile generation failed: {e}")))?;

    Ok(response.text_or_empty().trim().to_string())
}

async fn generate_compose<C: LlmConnector>(
    connector: &Arc<C>,
    language: &Language,
    dockerfile_content: &str,
    deps: &DependencyReport,
) -> Result<String, CoreError> {
    let prompt = format!(
        r#"Generate a docker-compose.yml for a {language} application that uses the following Dockerfile.

## Dockerfile:
```dockerfile
{dockerfile_content}
```

## Dependencies detected: {dep_count} package(s)

Requirements:
- Use `compose.yml` / Compose Specification format (version-less)
- Define a `app` service built from the local Dockerfile
- Add any common companion services the language ecosystem requires
  (e.g. a database, cache, or message broker) if the dependency list suggests them
- Use environment variables with sensible defaults for secrets
- Mount a named volume for any stateful data
- Expose the appropriate port(s)

Return ONLY the docker-compose.yml content — no markdown fences, no explanation."#,
        dep_count = deps.dependencies.len(),
    );

    let request = LlmRequestBuilder::new()
        .system("You are a DevOps expert. Return only the requested file content with no surrounding explanation or markdown.")
        .user(prompt)
        .max_tokens(1024)
        .build();

    let response = connector
        .complete(request)
        .await
        .map_err(|e| CoreError::other(format!("LLM docker-compose generation failed: {e}")))?;

    Ok(response.text_or_empty().trim().to_string())
}

async fn diagnose_and_fix<C: LlmConnector>(
    connector: &Arc<C>,
    language: &Language,
    current_dockerfile: &str,
    build_error: &str,
) -> Result<String, CoreError> {
    let prompt = format!(
        r#"A Docker build failed for a {language} application. Diagnose the error and return a corrected Dockerfile.

## Current Dockerfile:
```dockerfile
{current_dockerfile}
```

## Build error output:
```
{error}
```

Analyse the root cause and return a corrected Dockerfile.
Return ONLY the Dockerfile content — no markdown fences, no explanation."#,
        error = &build_error[..build_error.len().min(3000)],
    );

    let request = LlmRequestBuilder::new()
        .system("You are a Docker expert. Diagnose and fix the Dockerfile. Return only the corrected Dockerfile with no surrounding text.")
        .user(prompt)
        .max_tokens(2048)
        .build();

    let response = connector
        .complete(request)
        .await
        .map_err(|e| CoreError::other(format!("LLM Dockerfile fix failed: {e}")))?;

    Ok(response.text_or_empty().trim().to_string())
}

// ── Docker helpers (using CLI to avoid bollard body-type complexity) ──────────

/// Shallow-clone a repo into `dest`. Avoids full history for speed.
async fn shallow_clone(repo_url: &str, dest: &Path) -> Result<(), CoreError> {
    let output = tokio::process::Command::new("git")
        .args([
            "clone",
            "--depth=1",
            "--quiet",
            repo_url,
            dest.to_str().unwrap_or("."),
        ])
        .output()
        .await
        .map_err(|e| CoreError::other(format!("git clone failed to spawn: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CoreError::other(format!("git clone failed: {stderr}")));
    }
    Ok(())
}

/// Run `docker build` in `context_dir`, tagging the image as `tag`.
/// Returns the combined stdout+stderr on failure for LLM diagnosis.
async fn docker_build(context_dir: &Path, tag: &str) -> Result<(), String> {
    let output = tokio::process::Command::new("docker")
        .args([
            "build",
            "--no-cache",
            "--progress=plain",
            "-t",
            tag,
            context_dir.to_str().unwrap_or("."),
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to spawn docker build: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("STDOUT:\n{stdout}\nSTDERR:\n{stderr}"))
    }
}

/// Remove a Docker image (best-effort, ignores errors).
async fn docker_rmi(tag: &str) -> Result<(), String> {
    tokio::process::Command::new("docker")
        .args(["rmi", "--force", tag])
        .output()
        .await
        .map_err(|e| format!("docker rmi failed: {e}"))?;
    Ok(())
}

// ── Language heuristics ───────────────────────────────────────────────────────

/// Returns language-specific Dockerfile best-practice hints.
fn language_dockerfile_hints(lang: &Language) -> &'static str {
    match lang {
        Language::Rust => {
            "- Use a two-stage build: `rust:1-slim` builder → `debian:bookworm-slim` runner\n\
             - Cache dependencies by copying Cargo.toml/Cargo.lock and doing a dummy build first\n\
             - Strip the binary (`--release` + optional `strip`)\n\
             - Default port 8080 if an HTTP server is detected"
        }
        Language::Node => {
            "- Use `node:20-alpine` for both build and run stages\n\
             - Run `npm ci --only=production` (not `npm install`)\n\
             - Copy only the built output and node_modules into the final stage\n\
             - Default port 3000"
        }
        Language::Python => {
            "- Use `python:3.12-slim` as the base\n\
             - Install dependencies with `pip install --no-cache-dir -r requirements.txt` or `pip install .`\n\
             - Set `PYTHONDONTWRITEBYTECODE=1` and `PYTHONUNBUFFERED=1`\n\
             - Use `gunicorn` or `uvicorn` as the CMD for web apps\n\
             - Default port 8000"
        }
        Language::Go => {
            "- Two-stage build: `golang:1.22-alpine` builder → `alpine:3.19` runner\n\
             - Set `CGO_ENABLED=0 GOOS=linux` for a static binary\n\
             - Copy only the compiled binary into the final stage\n\
             - Default port 8080"
        }
        Language::Java => {
            "- Two-stage build: `eclipse-temurin:21-jdk` builder → `eclipse-temurin:21-jre-alpine` runner\n\
             - Use Maven or Gradle wrapper (`./mvnw` or `./gradlew`) to build\n\
             - Copy only the JAR into the final stage\n\
             - Default port 8080"
        }
        Language::Ruby => {
            "- Use `ruby:3.3-slim` as the base\n\
             - Run `bundle install --without development test`\n\
             - Use `puma` as the default server\n\
             - Default port 3000"
        }
        Language::PHP => {
            "- Use `php:8.3-fpm-alpine` with nginx as a separate service\n\
             - Run `composer install --no-dev --optimize-autoloader`\n\
             - Default port 9000 (FPM)"
        }
        _ => "- Use an appropriate official base image\n- Pin to a specific version tag",
    }
}

/// Returns true if this repo is likely a service (HTTP server, worker, etc.)
/// rather than a pure library, based on language and dependency signals.
fn looks_like_service(lang: &Language, deps: &DependencyReport) -> bool {
    let dep_names: Vec<String> =
        deps.dependencies.iter().map(|d| d.name.to_lowercase()).collect();

    match lang {
        Language::Rust => dep_names.iter().any(|d| {
            ["axum", "actix-web", "warp", "rocket", "poem", "tide"].contains(&d.as_str())
        }),
        Language::Node => dep_names.iter().any(|d| {
            ["express", "fastify", "koa", "hapi", "nestjs", "next", "nuxt"]
                .iter()
                .any(|s| d.contains(s))
        }),
        Language::Python => dep_names.iter().any(|d| {
            ["flask", "django", "fastapi", "starlette", "tornado", "uvicorn"]
                .iter()
                .any(|s| d.contains(s))
        }),
        Language::Go => dep_names.iter().any(|d| d.contains("gin") || d.contains("echo") || d.contains("chi")),
        Language::Java => dep_names.iter().any(|d| d.contains("spring") || d.contains("quarkus") || d.contains("micronaut")),
        // Libraries in these languages are less common as containerised services.
        Language::Ruby => true, // Rails apps are almost always services
        _ => false,
    }
}
