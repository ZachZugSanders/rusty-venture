# rusty-venture-actions/src/repo

## Purpose
All repository analysis actions and the maturity scoring model. This is the analytical core of the product.

## Action inventory

| File | Action | CTX key written | Tier |
|---|---|---|---|
| `clone.rs` | `CloneRepoAction` | `CTX_REPO_LOCAL_PATH` | 1 |
| `detect_language.rs` | `DetectLanguageAction` | `CTX_DETECTED_LANGUAGES` | 1 |
| `analyze_deps.rs` | `AnalyzeDepsAction` | `CTX_DEPENDENCY_REPORT` | 1 |
| `find_dockerfiles.rs` | `FindDockerfilesAction` | `CTX_DOCKERFILE_REPORT` | 1 |
| `audit_files.rs` | `AuditCommittedFilesAction` | `CTX_AUDIT_REPORT` | 1 |
| `governance.rs` | `GovernanceCheckAction` | `CTX_GOVERNANCE_REPORT` | 1 |
| `detect_llm.rs` | `DetectLlmConfigAction` | `CTX_LLM_CONFIG` | 1* |
| `content_quality.rs` | `ContentQualityCheckAction` | `CTX_CONTENT_QUALITY_REPORT` | 2 |
| `active_validation.rs` | `ActiveValidationAction` | `CTX_ACTIVE_VALIDATION_REPORT` | 3 |
| `report.rs` | `GenerateReportAction` | `CTX_FINAL_REPORT` | all |
| `maturity.rs` | (pure function, no action) | `CTX_MATURITY_SCORE` | all |

*`detect_llm` runs in the analysis container but gates tier 3 unlock signals.

## Three-phase pipeline (defined in `mod.rs`)
```
Phase 1 (sequential):  spawn-clone → clone-repo → [cache-image] → spawn-analysis
Phase 2 (parallel DAG): detect-language ┐
                        analyze-deps    ├─ all concurrent, write disjoint CTX keys
                        find-dockerfiles├
                        audit-files     ├
                        governance-check├
                        detect-llm-config┘
Phase 3 (sequential):  [content-quality] → [spawn-exec → active-validation] → generate-report → cleanup
```

## Maturity scoring (`maturity.rs`)
Six weighted dimensions, each scored 0–100:
- **Security** (25%) — audit violations, security policy, dep update automation
- **Dependency Health** (20%) — lock file, manifest completeness, pinned images
- **Build & CI** (15%) — CI config, reproducible builds, Docker best practices
- **Code Organization** (15%) — language idioms, MSRV, lint config, LLM context coverage
- **Project Governance** (15%) — LICENSE, README, CHANGELOG, LLM config presence
- **Testing & Quality** (10%) — test files, SAST, coverage config

Composite = weighted average → mapped to Bronze/Silver/Gold/Platinum/Diamond.

## Tier 3 unlock requirement
The project being scanned must have a `.llm-config` or `.claude/` directory detected by `DetectLlmConfigAction`. Without this, tier 3 `active-validation` is not meaningful to score because there's no LLM context to validate against.

## LLM provider enum (`detect_llm.rs`)
`LlmProvider`: Claude, OpenAi, Gemini, Copilot, Cursor, Grok, Llama, Mistral, DeepSeek, AwsBedrock, AzureOpenAi, Custom.

Project config format (`.llm-config` or `llm.toml`):
```
LLM=CLAUDE
MODEL=claude-sonnet-4-6
```
