//! Repository Change pipeline — apply scan suggestions to a cloned repo.
//!
//! ## Conceptual flow
//!
//! ```text
//! 1. Collect repair actions (each declares which files it touches).
//! 2. ChangeScheduler groups them: disjoint → Parallel batch, overlapping → Sequential batch.
//! 3. ApplyChangesWorkflow drives the git branching strategy:
//!    - Parallel batch: one branch per action, concurrent apply, LLM-assisted merge back.
//!    - Sequential batch: commit directly to the base branch in order.
//! ```
//!
//! See [`repair::RepairAction`], [`scheduler::ChangeScheduler`], and
//! [`apply::ApplyChangesWorkflow`] for the full API.

pub mod apply;
pub mod git;
pub mod merge;
pub mod repair;
pub mod scheduler;

pub use apply::{AppliedChange, ApplyChangesWorkflow, ChangeStatus, CTX_APPLIED_CHANGES};
pub use git::ContainerGit;
pub use repair::RepairAction;
pub use scheduler::{BatchKind, ChangeScheduler, SchedulePlan};
