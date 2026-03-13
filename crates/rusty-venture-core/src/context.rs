use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::CoreError;

/// A typed heterogeneous key-value store shared across all steps in a workflow.
/// Steps communicate by inserting and reading named values via string keys.
/// Access is guarded by an async RwLock.
#[derive(Clone)]
pub struct ExecutionContext {
    store: Arc<RwLock<HashMap<String, Box<dyn Any + Send + Sync>>>>,
    pub run_id: Uuid,
    pub workflow_name: String,
}

impl ExecutionContext {
    pub fn new(workflow_name: impl Into<String>) -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            run_id: Uuid::new_v4(),
            workflow_name: workflow_name.into(),
        }
    }

    /// Insert a typed value under a string key, overwriting any previous value.
    pub async fn insert<T: Any + Send + Sync + 'static>(
        &self,
        key: impl Into<String>,
        value: T,
    ) {
        let mut store = self.store.write().await;
        store.insert(key.into(), Box::new(value));
    }

    /// Retrieve a cloned typed value. Returns `None` if the key is absent or
    /// the stored type does not match `T`.
    pub async fn get<T: Any + Send + Sync + Clone + 'static>(&self, key: &str) -> Option<T> {
        let store = self.store.read().await;
        store
            .get(key)
            .and_then(|v| v.downcast_ref::<T>())
            .cloned()
    }

    /// Like `get` but returns an error if the key is absent or type mismatches.
    pub async fn require<T: Any + Send + Sync + Clone + 'static>(
        &self,
        key: &str,
    ) -> Result<T, CoreError> {
        self.get::<T>(key).await.ok_or_else(|| CoreError::ContextKeyNotFound {
            key: key.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn insert_and_get_typed_value() {
        let ctx = ExecutionContext::new("test-workflow");
        ctx.insert("my_key", 42u32).await;
        let val: Option<u32> = ctx.get("my_key").await;
        assert_eq!(val, Some(42));
    }

    #[tokio::test]
    async fn get_missing_key_returns_none() {
        let ctx = ExecutionContext::new("test-workflow");
        let val: Option<String> = ctx.get("nope").await;
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn get_wrong_type_returns_none() {
        let ctx = ExecutionContext::new("test-workflow");
        ctx.insert("key", "hello".to_string()).await;
        let val: Option<u32> = ctx.get("key").await;
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn require_returns_error_on_missing_key() {
        let ctx = ExecutionContext::new("test-workflow");
        let result: Result<String, _> = ctx.require("missing").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn overwrite_value() {
        let ctx = ExecutionContext::new("test-workflow");
        ctx.insert("k", 1u32).await;
        ctx.insert("k", 2u32).await;
        let val: Option<u32> = ctx.get("k").await;
        assert_eq!(val, Some(2));
    }
}
