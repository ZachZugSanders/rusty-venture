use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::task::JoinSet;
use tracing::{error, info, warn};

use crate::{
    context::ExecutionContext,
    error::CoreError,
    workflow::{execute_step, OnFailure, Step},
};

// ── DagNode ───────────────────────────────────────────────────────────────────

/// A node in a DAG workflow: a step plus the names of steps it depends on.
///
/// A node only starts executing once **all** nodes listed in `depends_on` have
/// completed (successfully or with a non-Abort `OnFailure` policy).
pub struct DagNode {
    /// The step to execute when this node becomes ready.
    pub(crate) step: Arc<Step>,
    /// Names of nodes that must complete before this one starts.
    pub depends_on: Vec<String>,
}

impl DagNode {
    /// Create a node with no dependencies.
    pub fn new(step: Step) -> Self {
        Self {
            step: Arc::new(step),
            depends_on: vec![],
        }
    }

    /// Declare that this node depends on the listed node names.
    pub fn depends_on(mut self, deps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.depends_on = deps.into_iter().map(Into::into).collect();
        self
    }
}

// ── DagWorkflow ───────────────────────────────────────────────────────────────

/// A directed acyclic graph of steps. Nodes with no unmet dependencies execute
/// in parallel; downstream nodes unlock as their prerequisites finish.
pub struct DagWorkflow {
    pub name: String,
    nodes: HashMap<String, DagNode>,
}

// ── DagWorkflowBuilder ────────────────────────────────────────────────────────

pub struct DagWorkflowBuilder {
    name: String,
    nodes: HashMap<String, DagNode>,
}

impl DagWorkflowBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            nodes: HashMap::new(),
        }
    }

    /// Add a node. The node's step name is used as its key in the graph.
    pub fn node(mut self, node: DagNode) -> Self {
        let key = node.step.name.clone();
        self.nodes.insert(key, node);
        self
    }

    pub fn build(self) -> DagWorkflow {
        DagWorkflow {
            name: self.name,
            nodes: self.nodes,
        }
    }
}

// ── DagEngine ─────────────────────────────────────────────────────────────────

/// Executes a `DagWorkflow`, running independent nodes in parallel via Tokio.
///
/// Execution order:
/// 1. Nodes with no dependencies start immediately (potentially all in parallel).
/// 2. As each node finishes, nodes whose last unmet dependency just cleared are
///    spawned.
/// 3. If a node's `OnFailure` is `Abort`, in-flight tasks are cancelled and an
///    error is returned. `Continue` and `LlmRemediate` nodes still unlock their
///    dependents so the rest of the DAG can proceed.
pub struct DagEngine;

/// Internal result type threaded through the JoinSet.
struct NodeResult {
    name: String,
    on_failure: OnFailure,
    outcome: Result<(), CoreError>,
}

impl DagEngine {
    pub fn new() -> Self {
        DagEngine
    }

    pub async fn run(dag: DagWorkflow, ctx: &ExecutionContext) -> Result<(), CoreError> {
        info!(
            dag = %dag.name,
            run_id = %ctx.run_id,
            nodes = dag.nodes.len(),
            "Starting DAG workflow"
        );

        // ── Validate structure ─────────────────────────────────────────────
        Self::validate(&dag)?;

        // ── Build execution graph ──────────────────────────────────────────
        //
        // in_degree[A]    = number of A's unmet prerequisites
        // reverse_adj[B]  = nodes that become ready when B finishes
        let mut in_degree: HashMap<String, usize> = dag
            .nodes
            .keys()
            .map(|k| (k.clone(), 0usize))
            .collect();

        let mut reverse_adj: HashMap<String, Vec<String>> = HashMap::new();

        for (name, node) in &dag.nodes {
            for dep in &node.depends_on {
                *in_degree.get_mut(name).unwrap() += 1;
                reverse_adj.entry(dep.clone()).or_default().push(name.clone());
            }
        }

        // Extract Arc<Step> map — we clone Arcs into spawned tasks.
        let steps: HashMap<String, Arc<Step>> = dag
            .nodes
            .into_iter()
            .map(|(name, node)| (name, node.step))
            .collect();

        // ── Seed: spawn all initially-ready nodes ──────────────────────────
        let mut join_set: JoinSet<NodeResult> = JoinSet::new();

        for (name, &degree) in &in_degree {
            if degree == 0 {
                Self::spawn_node(name, &steps, ctx, &mut join_set);
            }
        }

        // ── Drive the DAG to completion ────────────────────────────────────
        while let Some(join_result) = join_set.join_next().await {
            let NodeResult { name, on_failure, outcome } =
                join_result.map_err(|e| CoreError::other(format!("DAG task panicked: {e}")))?;

            let node_succeeded = match outcome {
                Ok(()) => {
                    info!(node = %name, "DAG node succeeded");
                    true
                }
                Err(e) => match on_failure {
                    OnFailure::Abort => {
                        error!(node = %name, error = %e, "DAG node failed, aborting");
                        join_set.abort_all();
                        return Err(CoreError::StepFailed {
                            step: name,
                            attempts: 1,
                            message: e.to_string(),
                        });
                    }
                    OnFailure::Continue | OnFailure::LlmRemediate { .. } => {
                        warn!(node = %name, error = %e, "DAG node failed, unlocking dependents");
                        // Failure recorded in context by execute_step already.
                        true
                    }
                },
            };

            if node_succeeded {
                // Unlock nodes whose last dependency just cleared.
                if let Some(successors) = reverse_adj.get(&name) {
                    for successor in successors {
                        let deg = in_degree.get_mut(successor).unwrap();
                        *deg -= 1;
                        if *deg == 0 {
                            Self::spawn_node(successor, &steps, ctx, &mut join_set);
                        }
                    }
                }
            }
        }

        info!(dag = %dag.name, run_id = %ctx.run_id, "DAG workflow completed");
        Ok(())
    }

    fn spawn_node(
        name: &str,
        steps: &HashMap<String, Arc<Step>>,
        ctx: &ExecutionContext,
        join_set: &mut JoinSet<NodeResult>,
    ) {
        let step = Arc::clone(steps.get(name).expect("node missing from steps map"));
        let ctx = ctx.clone();
        let on_failure = step.on_failure.clone();
        let node_name = name.to_string();

        info!(node = %node_name, "Spawning DAG node");

        join_set.spawn(async move {
            let outcome = execute_step(&step, &ctx).await;
            NodeResult {
                name: node_name,
                on_failure,
                outcome,
            }
        });
    }

    /// Validate the DAG before execution:
    /// - All names referenced in `depends_on` must exist as nodes.
    /// - The graph must be acyclic (detected via Kahn's algorithm).
    fn validate(dag: &DagWorkflow) -> Result<(), CoreError> {
        // Check for unknown dependency references.
        for (name, node) in &dag.nodes {
            for dep in &node.depends_on {
                if !dag.nodes.contains_key(dep.as_str()) {
                    return Err(CoreError::other(format!(
                        "DAG node '{name}' depends on unknown node '{dep}'"
                    )));
                }
            }
        }

        // Cycle detection via Kahn's algorithm.
        // in_deg[A] = A.depends_on.len() (number of unmet prerequisites).
        let mut in_deg: HashMap<&str, usize> = dag
            .nodes
            .iter()
            .map(|(k, v)| (k.as_str(), v.depends_on.len()))
            .collect();

        // reverse_adj[B] = nodes A where A.depends_on contains B.
        let mut reverse_adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for (name, node) in &dag.nodes {
            for dep in &node.depends_on {
                reverse_adj.entry(dep.as_str()).or_default().push(name.as_str());
            }
        }

        let mut queue: VecDeque<&str> = in_deg
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&n, _)| n)
            .collect();

        let mut processed = 0usize;
        while let Some(node) = queue.pop_front() {
            processed += 1;
            if let Some(successors) = reverse_adj.get(node) {
                for &succ in successors {
                    let d = in_deg.get_mut(succ).unwrap();
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(succ);
                    }
                }
            }
        }

        if processed != dag.nodes.len() {
            return Err(CoreError::other(
                "DAG workflow contains a cycle — check depends_on declarations",
            ));
        }

        Ok(())
    }
}

impl Default for DagEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;

    use super::*;
    use crate::{
        action::Action,
        workflow::{OnFailure, StepBuilder},
        ExecutionContext,
    };

    // A minimal action that records how many times it ran.
    struct CounterAction {
        key: String,
        counter: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Action for CounterAction {
        type Input = ();
        type Output = ();

        fn name(&self) -> &str {
            &self.key
        }

        async fn execute(&self, ctx: &ExecutionContext, _: ()) -> Result<(), CoreError> {
            self.counter.fetch_add(1, Ordering::SeqCst);
            ctx.insert(self.key.clone(), true).await;
            Ok(())
        }
    }

    fn counter_node(
        name: &str,
        counter: Arc<AtomicUsize>,
        deps: &[&str],
    ) -> DagNode {
        let step = StepBuilder::<(), ()>::new(name)
            .action(CounterAction {
                key: name.to_string(),
                counter,
            })
            .on_failure(OnFailure::Abort)
            .build();
        DagNode::new(step).depends_on(deps.iter().map(|s| s.to_string()))
    }

    #[tokio::test]
    async fn independent_nodes_all_run() {
        let c1 = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::new(AtomicUsize::new(0));
        let c3 = Arc::new(AtomicUsize::new(0));

        let dag = DagWorkflowBuilder::new("test-parallel")
            .node(counter_node("a", Arc::clone(&c1), &[]))
            .node(counter_node("b", Arc::clone(&c2), &[]))
            .node(counter_node("c", Arc::clone(&c3), &[]))
            .build();

        let ctx = ExecutionContext::new("test");
        DagEngine::run(dag, &ctx).await.unwrap();

        assert_eq!(c1.load(Ordering::SeqCst), 1);
        assert_eq!(c2.load(Ordering::SeqCst), 1);
        assert_eq!(c3.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dependent_node_runs_after_prerequisite() {
        let order: Arc<tokio::sync::Mutex<Vec<String>>> =
            Arc::new(tokio::sync::Mutex::new(vec![]));

        struct OrderedAction {
            key: String,
            order: Arc<tokio::sync::Mutex<Vec<String>>>,
        }

        #[async_trait]
        impl Action for OrderedAction {
            type Input = ();
            type Output = ();
            fn name(&self) -> &str { &self.key }
            async fn execute(&self, ctx: &ExecutionContext, _: ()) -> Result<(), CoreError> {
                order_push(&self.order, &self.key).await;
                ctx.insert(self.key.clone(), true).await;
                Ok(())
            }
        }

        async fn order_push(order: &Arc<tokio::sync::Mutex<Vec<String>>>, key: &str) {
            order.lock().await.push(key.to_string());
        }

        let o1 = Arc::clone(&order);
        let o2 = Arc::clone(&order);
        let o3 = Arc::clone(&order);

        let make = |name: &str, order: Arc<tokio::sync::Mutex<Vec<String>>>, deps: &[&str]| {
            let step = StepBuilder::<(), ()>::new(name)
                .action(OrderedAction { key: name.to_string(), order })
                .build();
            DagNode::new(step).depends_on(deps.iter().map(|s| s.to_string()))
        };

        let dag = DagWorkflowBuilder::new("ordering-test")
            .node(make("root", o1, &[]))
            .node(make("mid", o2, &["root"]))
            .node(make("leaf", o3, &["mid"]))
            .build();

        let ctx = ExecutionContext::new("test");
        DagEngine::run(dag, &ctx).await.unwrap();

        let result = order.lock().await;
        assert_eq!(*result, vec!["root", "mid", "leaf"]);
    }

    #[tokio::test]
    async fn cycle_is_rejected() {
        let step_a = StepBuilder::<(), ()>::new("a")
            .action(CounterAction { key: "a".into(), counter: Arc::new(AtomicUsize::new(0)) })
            .build();
        let step_b = StepBuilder::<(), ()>::new("b")
            .action(CounterAction { key: "b".into(), counter: Arc::new(AtomicUsize::new(0)) })
            .build();

        let dag = DagWorkflowBuilder::new("cycle-test")
            .node(DagNode::new(step_a).depends_on(["b"]))
            .node(DagNode::new(step_b).depends_on(["a"]))
            .build();

        let ctx = ExecutionContext::new("test");
        let result = DagEngine::run(dag, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cycle"));
    }

    #[tokio::test]
    async fn unknown_dependency_is_rejected() {
        let step = StepBuilder::<(), ()>::new("a")
            .action(CounterAction { key: "a".into(), counter: Arc::new(AtomicUsize::new(0)) })
            .build();

        let dag = DagWorkflowBuilder::new("unknown-dep-test")
            .node(DagNode::new(step).depends_on(["nonexistent"]))
            .build();

        let ctx = ExecutionContext::new("test");
        let result = DagEngine::run(dag, &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown node"));
    }
}
