use crate::{context::ExecutionContext, error::CoreError};
use async_trait::async_trait;
use std::any::Any;

/// The fundamental unit of work. Every action is async, receives a shared
/// execution context and its typed input, and produces a typed output.
#[async_trait]
pub trait Action: Send + Sync {
    type Input: Send + Sync + 'static;
    type Output: Send + Sync + 'static;

    /// Human-readable name used in logs and reports.
    fn name(&self) -> &str;

    /// Execute the action.
    async fn execute(
        &self,
        ctx: &ExecutionContext,
        input: Self::Input,
    ) -> Result<Self::Output, CoreError>;
}

/// Type-erased version of `Action`. Used internally by the workflow engine to
/// hold heterogeneous actions in a `Vec`. Users never implement this directly —
/// a blanket impl handles the conversion from any `Action`.
#[async_trait]
pub trait AnyAction: Send + Sync {
    fn name(&self) -> &str;

    async fn execute_erased(
        &self,
        ctx: &ExecutionContext,
        input: Box<dyn Any + Send + Sync>,
    ) -> Result<Box<dyn Any + Send + Sync>, CoreError>;
}

/// Blanket impl: any `Action<Input=I, Output=O>` where I and O are `'static`
/// automatically becomes an `AnyAction`.
#[async_trait]
impl<A> AnyAction for A
where
    A: Action + Send + Sync,
    A::Input: Any + Send + Sync + 'static,
    A::Output: Any + Send + Sync + 'static,
{
    fn name(&self) -> &str {
        Action::name(self)
    }

    async fn execute_erased(
        &self,
        ctx: &ExecutionContext,
        input: Box<dyn Any + Send + Sync>,
    ) -> Result<Box<dyn Any + Send + Sync>, CoreError> {
        let typed_input = input
            .downcast::<A::Input>()
            .map_err(|_| CoreError::TypeMismatch {
                action: self.name().to_string(),
                expected: std::any::type_name::<A::Input>().to_string(),
            })?;

        let output = Action::execute(self, ctx, *typed_input).await?;
        Ok(Box::new(output) as Box<dyn Any + Send + Sync>)
    }
}
