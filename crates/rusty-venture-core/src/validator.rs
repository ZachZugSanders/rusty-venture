use crate::error::CoreError;
use async_trait::async_trait;

/// The outcome of a validation check on a step's output.
#[derive(Debug)]
pub enum ValidationOutcome {
    /// All checks passed.
    Passed,

    /// Validation failed with a human-readable reason and optional structured context
    /// that can be forwarded to the LLM for remediation suggestions.
    Failed {
        reason: String,
        detail: Option<serde_json::Value>,
    },

    /// Validator encountered an internal error (distinct from a logical failure).
    Error(CoreError),
}

impl ValidationOutcome {
    pub fn is_passed(&self) -> bool {
        matches!(self, ValidationOutcome::Passed)
    }

    pub fn failure_reason(&self) -> Option<&str> {
        match self {
            ValidationOutcome::Failed { reason, .. } => Some(reason.as_str()),
            _ => None,
        }
    }
}

/// A validator is attached to a Step. After the action succeeds, all validators
/// for that step run in order. On failure, the Step's `OnFailure` policy is applied.
#[async_trait]
pub trait Validator<T>: Send + Sync {
    fn name(&self) -> &str;
    async fn validate(&self, value: &T) -> ValidationOutcome;
}

/// A simple non-empty string validator.
pub struct NonEmptyValidator {
    field_name: String,
}

impl NonEmptyValidator {
    pub fn new(field_name: impl Into<String>) -> Self {
        Self {
            field_name: field_name.into(),
        }
    }
}

#[async_trait]
impl Validator<String> for NonEmptyValidator {
    fn name(&self) -> &str {
        "non-empty"
    }

    async fn validate(&self, value: &String) -> ValidationOutcome {
        if value.trim().is_empty() {
            ValidationOutcome::Failed {
                reason: format!("Field '{}' must not be empty", self.field_name),
                detail: None,
            }
        } else {
            ValidationOutcome::Passed
        }
    }
}
