use std::collections::HashSet;

use super::repair::RepairAction;

// ── Public types ──────────────────────────────────────────────────────────────

/// The execution plan produced by `ChangeScheduler::schedule`.
///
/// Batches are ordered: each batch must finish before the next begins.
/// Within a `Parallel` batch every action can run concurrently on its own
/// git branch.  Within a `Sequential` batch actions run one after another on
/// the shared base branch.
pub struct SchedulePlan {
    pub batches: Vec<Batch>,
}

pub struct Batch {
    pub kind: BatchKind,
    pub actions: Vec<Box<dyn RepairAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchKind {
    /// Run actions one after another on the main improvement branch.
    Sequential,
    /// Run each action on its own branch, then merge all back.
    Parallel,
}

// ── ChangeScheduler ───────────────────────────────────────────────────────────

/// Groups [`RepairAction`]s into sequential or parallel batches based on
/// whether their declared `affected_files()` globs overlap.
///
/// ## Algorithm
/// 1. Partition actions whose file sets are pairwise disjoint into one
///    `Parallel` batch.
/// 2. Actions that overlap with any already-scheduled action go into a
///    `Sequential` batch run after the parallel group.
/// 3. If only one action exists in what would be a parallel group, it is
///    emitted as a single-action `Sequential` batch (no branching overhead).
///
/// This is a greedy single-pass algorithm.  For most repo-improvement
/// workloads (5–20 actions) it is both fast and produces a good plan.
pub struct ChangeScheduler;

impl ChangeScheduler {
    /// Schedule `actions` into a `SchedulePlan`.
    pub fn schedule(actions: Vec<Box<dyn RepairAction>>) -> SchedulePlan {
        let mut parallel: Vec<Box<dyn RepairAction>> = vec![];
        let mut sequential: Vec<Box<dyn RepairAction>> = vec![];

        // Track the combined glob set for the parallel group.
        let mut parallel_globs: HashSet<String> = HashSet::new();

        for action in actions {
            let globs: HashSet<String> = action.affected_files().into_iter().collect();

            if Self::overlaps(&globs, &parallel_globs) {
                // This action touches files already claimed by a parallel action
                // → must run sequentially after the parallel batch.
                sequential.push(action);
            } else {
                // Disjoint with everything in parallel so far → can run in
                // parallel.
                parallel_globs.extend(globs);
                parallel.push(action);
            }
        }

        let mut batches: Vec<Batch> = vec![];

        // Emit the parallel batch (or collapse to sequential if only one action).
        if parallel.len() == 1 {
            batches.push(Batch {
                kind: BatchKind::Sequential,
                actions: parallel,
            });
        } else if !parallel.is_empty() {
            batches.push(Batch {
                kind: BatchKind::Parallel,
                actions: parallel,
            });
        }

        // Emit the sequential batch.
        if !sequential.is_empty() {
            batches.push(Batch {
                kind: BatchKind::Sequential,
                actions: sequential,
            });
        }

        SchedulePlan { batches }
    }

    /// Returns true if the two glob sets have any pattern in common.
    ///
    /// Currently uses exact string equality for patterns — good enough for
    /// detecting obvious overlaps like two actions both declaring `Cargo.toml`
    /// or `**/*.rs`.  Future improvement: full glob intersection.
    fn overlaps(a: &HashSet<String>, b: &HashSet<String>) -> bool {
        // Exact match check.
        if a.intersection(b).next().is_some() {
            return true;
        }

        // Wildcard containment: if either set contains `**` or a catch-all,
        // any non-empty other set overlaps.
        let catch_all = |s: &HashSet<String>| s.iter().any(|g| g == "**" || g == "**/*");
        if catch_all(a) && !b.is_empty() {
            return true;
        }
        if catch_all(b) && !a.is_empty() {
            return true;
        }

        // Prefix overlap: `src/**` overlaps with `src/main.rs`.
        for ga in a {
            let prefix_a = ga.trim_end_matches("/**").trim_end_matches("/*");
            for gb in b {
                let prefix_b = gb.trim_end_matches("/**").trim_end_matches("/*");
                if prefix_a.starts_with(prefix_b) || prefix_b.starts_with(prefix_a) {
                    return true;
                }
            }
        }

        false
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use rusty_venture_core::{context::ExecutionContext, CoreError};

    use crate::change::git::ContainerGit;

    struct FakeRepair {
        name: String,
        files: Vec<String>,
    }

    #[async_trait]
    impl RepairAction for FakeRepair {
        fn name(&self) -> &str { &self.name }
        fn affected_files(&self) -> Vec<String> { self.files.clone() }
        async fn apply(&self, _git: &ContainerGit, _ctx: &ExecutionContext) -> Result<(), CoreError> {
            Ok(())
        }
    }

    fn fake(name: &str, files: &[&str]) -> Box<dyn RepairAction> {
        Box::new(FakeRepair {
            name: name.to_string(),
            files: files.iter().map(|s| s.to_string()).collect(),
        })
    }

    #[test]
    fn disjoint_actions_are_parallel() {
        let plan = ChangeScheduler::schedule(vec![
            fake("add-license", &["LICENSE"]),
            fake("add-security", &["SECURITY.md"]),
            fake("add-contributing", &["CONTRIBUTING.md"]),
        ]);

        assert_eq!(plan.batches.len(), 1);
        assert_eq!(plan.batches[0].kind, BatchKind::Parallel);
        assert_eq!(plan.batches[0].actions.len(), 3);
    }

    #[test]
    fn overlapping_actions_are_sequential() {
        let plan = ChangeScheduler::schedule(vec![
            fake("format-rust", &["**/*.rs"]),
            fake("lint-rust", &["**/*.rs"]),
        ]);

        // Both touch `**/*.rs` → sequential.
        assert_eq!(plan.batches.len(), 2);
        assert_eq!(plan.batches[0].kind, BatchKind::Sequential);
        assert_eq!(plan.batches[1].kind, BatchKind::Sequential);
    }

    #[test]
    fn single_action_is_sequential() {
        let plan = ChangeScheduler::schedule(vec![fake("add-license", &["LICENSE"])]);

        assert_eq!(plan.batches.len(), 1);
        assert_eq!(plan.batches[0].kind, BatchKind::Sequential);
    }

    #[test]
    fn mixed_produces_parallel_then_sequential() {
        let plan = ChangeScheduler::schedule(vec![
            fake("add-license", &["LICENSE"]),
            fake("add-security", &["SECURITY.md"]),
            fake("update-readme", &["README.md"]),
            // This conflicts with update-readme
            fake("update-readme-2", &["README.md"]),
        ]);

        assert_eq!(plan.batches.len(), 2);
        assert_eq!(plan.batches[0].kind, BatchKind::Parallel);
        assert_eq!(plan.batches[0].actions.len(), 3); // license, security, readme
        assert_eq!(plan.batches[1].kind, BatchKind::Sequential);
        assert_eq!(plan.batches[1].actions.len(), 1); // readme-2
    }
}
