use std::any::Any;
use std::sync::Arc;

use tracing::{error, info, warn};

use crate::{
    action::{Action, AnyAction},
    context::ExecutionContext,
    error::CoreError,
    retry::RetryStrategy,
    validator::{ValidationOutcome, Validator},
};

/// What to do when a step's validators all fail after retries are exhausted.
#[derive(Clone, Debug, Default)]
pub enum OnFailure {
    /// Abort the workflow, returning an error.
    #[default]
    Abort,

    /// Record the failure but continue to the next step.
    Continue,

    /// Ask the LLM to suggest a remediation; the suggestion is stored in the
    /// context under `"llm_remediation.<step_name>"` and execution continues.
    LlmRemediate { context_prompt: String },
}

/// Type-erased validator holder so validators can be stored in a `Vec`.
#[allow(clippy::type_complexity)]
pub(crate) struct AnyValidatorBox {
    pub name: String,
    pub validate: Box<
        dyn Fn(
                &(dyn Any + Send + Sync),
            )
                -> std::pin::Pin<Box<dyn std::future::Future<Output = ValidationOutcome> + Send>>
            + Send
            + Sync,
    >,
}

/// A single step in a workflow pipeline.
pub struct Step {
    pub name: String,
    pub(crate) action: Arc<dyn AnyAction>,
    pub(crate) validators: Vec<AnyValidatorBox>,
    pub(crate) retry: RetryStrategy,
    pub(crate) on_failure: OnFailure,
    /// The concrete input value boxed as `Any`. Set at construction time.
    #[allow(dead_code)]
    pub(crate) input: Box<dyn Any + Send + Sync>,
}

/// Builder for a `Step` with a statically-typed action.
pub struct StepBuilder<I, O> {
    name: String,
    action: Option<Arc<dyn AnyAction>>,
    validators: Vec<AnyValidatorBox>,
    retry: RetryStrategy,
    on_failure: OnFailure,
    input: Option<Box<dyn Any + Send + Sync>>,
    _phantom: std::marker::PhantomData<(I, O)>,
}

impl<I, O> StepBuilder<I, O>
where
    I: Any + Send + Sync + 'static,
    O: Any + Send + Sync + 'static,
{
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            action: None,
            validators: vec![],
            retry: RetryStrategy::None,
            on_failure: OnFailure::Abort,
            input: None,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn action<A>(mut self, action: A) -> Self
    where
        A: Action<Input = I, Output = O> + 'static,
    {
        self.action = Some(Arc::new(action));
        self
    }

    /// Provide the input for this step. Steps that derive their input from the
    /// `ExecutionContext` should use `()` as input and read from `ctx` inside
    /// `Action::execute`.
    pub fn input(mut self, input: I) -> Self {
        self.input = Some(Box::new(input));
        self
    }

    /// Attach a validator that runs after this step succeeds. `O` must be
    /// `Clone` so the value can be moved into the async validation future.
    pub fn validate<V>(mut self, validator: V) -> Self
    where
        V: Validator<O> + 'static,
        O: Clone,
    {
        let name = validator.name().to_string();
        let validator = Arc::new(validator);
        let validate_fn = move |any: &(dyn Any + Send + Sync)| -> std::pin::Pin<
            Box<dyn std::future::Future<Output = ValidationOutcome> + Send>,
        > {
            let validator = Arc::clone(&validator);
            match any.downcast_ref::<O>() {
                Some(typed) => {
                    let value = typed.clone();
                    Box::pin(async move { validator.validate(&value).await })
                }
                None => Box::pin(async {
                    ValidationOutcome::Error(CoreError::other("type mismatch in validator"))
                }),
            }
        };
        self.validators.push(AnyValidatorBox {
            name,
            validate: Box::new(validate_fn),
        });
        self
    }

    pub fn retry(mut self, strategy: RetryStrategy) -> Self {
        self.retry = strategy;
        self
    }

    pub fn on_failure(mut self, policy: OnFailure) -> Self {
        self.on_failure = policy;
        self
    }

    pub fn build(self) -> Step {
        let action = self
            .action
            .expect("StepBuilder: action() must be called before build()");
        let input: Box<dyn Any + Send + Sync> = self
            .input
            .unwrap_or_else(|| Box::new(()) as Box<dyn Any + Send + Sync>);

        Step {
            name: self.name,
            action,
            validators: self.validators,
            retry: self.retry,
            on_failure: self.on_failure,
            input,
        }
    }
}

/// An ordered collection of steps that form a complete workflow.
pub struct Workflow {
    pub name: String,
    pub steps: Vec<Step>,
}

/// Fluent builder for a `Workflow`.
pub struct WorkflowBuilder {
    name: String,
    steps: Vec<Step>,
}

impl WorkflowBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: vec![],
        }
    }

    pub fn step(mut self, step: Step) -> Self {
        self.steps.push(step);
        self
    }

    pub fn build(self) -> Workflow {
        Workflow {
            name: self.name,
            steps: self.steps,
        }
    }
}

/// Executes a `Workflow` sequentially, handling retries and `OnFailure` policies.
pub struct WorkflowEngine;

impl WorkflowEngine {
    pub fn new() -> Self {
        WorkflowEngine
    }

    pub async fn run(&self, workflow: &Workflow, ctx: &ExecutionContext) -> Result<(), CoreError> {
        info!(
            workflow = %workflow.name,
            run_id = %ctx.run_id,
            steps = workflow.steps.len(),
            "Starting workflow"
        );
        ctx.emit_log(
            "info",
            None,
            format!(
                "Starting workflow '{}' — {} steps",
                workflow.name,
                workflow.steps.len()
            ),
        );

        for step in &workflow.steps {
            execute_step(step, ctx).await?;
        }

        info!(
            workflow = %workflow.name,
            run_id = %ctx.run_id,
            "Workflow completed successfully"
        );
        ctx.emit_log("info", None, "Workflow completed successfully");

        Ok(())
    }
}

impl Default for WorkflowEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ── Shared step execution logic ───────────────────────────────────────────────
//
// Used by both `WorkflowEngine` (sequential) and `DagEngine` (parallel).
// Handles retries, validators, and `OnFailure` policy dispatch.

pub(crate) async fn execute_step(step: &Step, ctx: &ExecutionContext) -> Result<(), CoreError> {
    let max_attempts = step.retry.max_attempts();
    let mut last_error: Option<CoreError> = None;

    for attempt in 1..=max_attempts {
        if attempt > 1 {
            if let Some(delay) = step.retry.delay_for(attempt) {
                info!(
                    step = %step.name,
                    attempt,
                    delay_ms = delay.as_millis(),
                    "Waiting before retry"
                );
                ctx.emit_log(
                    "info",
                    Some(&step.name),
                    format!(
                        "Retrying (attempt {attempt}/{max_attempts}) after {}ms",
                        delay.as_millis()
                    ),
                );
                tokio::time::sleep(delay).await;
            }
        }

        info!(step = %step.name, attempt, max_attempts, "Executing step");
        if attempt == 1 {
            ctx.emit_log(
                "info",
                Some(&step.name),
                format!("Starting step '{}'", step.name),
            );
        }

        // All actions read inputs from ExecutionContext using `()` as Input type.
        let input: Box<dyn Any + Send + Sync> = Box::new(());

        match step.action.execute_erased(ctx, input).await {
            Ok(output) => {
                let mut validation_failed = false;
                for v in &step.validators {
                    match (v.validate)(output.as_ref()).await {
                        ValidationOutcome::Passed => {
                            info!(step = %step.name, validator = %v.name, "Validator passed");
                        }
                        ValidationOutcome::Failed { reason, .. } => {
                            warn!(
                                step = %step.name,
                                validator = %v.name,
                                %reason,
                                "Validator failed"
                            );
                            last_error = Some(CoreError::StepFailed {
                                step: step.name.clone(),
                                attempts: attempt,
                                message: format!("validator '{}' failed: {}", v.name, reason),
                            });
                            validation_failed = true;
                            break;
                        }
                        ValidationOutcome::Error(e) => {
                            warn!(
                                step = %step.name,
                                validator = %v.name,
                                error = %e,
                                "Validator error"
                            );
                            last_error = Some(e);
                            validation_failed = true;
                            break;
                        }
                    }
                }

                if !validation_failed {
                    info!(step = %step.name, attempt, "Step succeeded");
                    ctx.emit_log(
                        "info",
                        Some(&step.name),
                        format!("Step '{}' completed", step.name),
                    );
                    return Ok(());
                }
            }
            Err(e) => {
                warn!(step = %step.name, attempt, error = %e, "Step attempt failed");
                ctx.emit_log(
                    "warn",
                    Some(&step.name),
                    format!("Step '{}' attempt {attempt} failed: {e}", step.name),
                );
                last_error = Some(e);
            }
        }
    }

    let err = last_error.unwrap_or_else(|| CoreError::other("unknown error"));
    let step_error = CoreError::StepFailed {
        step: step.name.clone(),
        attempts: max_attempts,
        message: err.to_string(),
    };

    match &step.on_failure {
        OnFailure::Abort => {
            error!(step = %step.name, "Step failed, aborting workflow");
            ctx.emit_log(
                "error",
                Some(&step.name),
                format!("Step '{}' failed — aborting workflow: {}", step.name, err),
            );
            Err(step_error)
        }
        OnFailure::Continue => {
            warn!(step = %step.name, "Step failed, continuing workflow");
            ctx.emit_log(
                "warn",
                Some(&step.name),
                format!("Step '{}' failed (continuing): {}", step.name, err),
            );
            ctx.insert(
                format!("step_failure.{}", step.name),
                step_error.to_string(),
            )
            .await;
            Ok(())
        }
        OnFailure::LlmRemediate { context_prompt } => {
            warn!(step = %step.name, "Step failed, requesting LLM remediation");
            ctx.emit_log(
                "warn",
                Some(&step.name),
                format!("Step '{}' failed — requesting LLM remediation", step.name),
            );
            ctx.insert(
                format!("llm_remediation_prompt.{}", step.name),
                context_prompt.clone(),
            )
            .await;
            ctx.insert(
                format!("step_failure.{}", step.name),
                step_error.to_string(),
            )
            .await;
            Ok(())
        }
    }
}
