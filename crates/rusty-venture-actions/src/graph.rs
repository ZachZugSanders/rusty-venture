//! Decision-graph transformation.
//!
//! Transforms a [`MaturityScore`] into a structured [`DecisionGraph`] payload
//! suitable for the 3D explorer.  The graph has three tiers of nodes:
//!
//! * **root** — one node representing the composite score.
//! * **dimension** — one node per maturity dimension.
//! * **signal** — one node per maturity signal inside each dimension.
//!
//! ## Position algorithm — hierarchical radial layout (distance-vector map)
//!
//! Nodes are positioned so that Euclidean distance in 3D space mirrors the
//! graph-theoretic relationship distance:
//!
//! * The **root** sits at the world origin `(0, 0, 0)`.
//! * **Dimensions** are evenly distributed on a circle of radius [`orbit_l1`]
//!   in the XZ plane so the inter-node distance reflects hierarchical depth.
//! * **Signals** fan out from their parent dimension on a sub-circle of radius
//!   [`orbit_l2`], oriented in the *tangential + vertical* plane so that signal
//!   clusters from neighbouring dimensions do not overlap.
//!
//! ## Node sizing
//!
//! The visual sphere radius for each node is pre-computed from
//! [`NodeSizeConfig`] — a static configuration struct with sensible defaults.
//! A future UI panel will expose this as interactive sliders; for now all
//! values are fixed at initialisation time.

use std::f32::consts::TAU;

use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::repo::maturity::MaturityScore;

// ── Node size / layout configuration ────────────────────────────────────────

/// Static configuration that controls the 3D layout and visual sphere sizes.
///
/// All numeric fields are intentionally plain `f32` public values so that a
/// future JSON serialisation round-trip or UI slider can override them without
/// requiring a builder or setter helpers.
///
/// The defaults produce a legible graph for a typical 4–8 dimension score with
/// 3–6 signals per dimension when rendered with `camera position [0, 4, 14]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSizeConfig {
    /// Orbital radius for dimension nodes (root → dimension distance).
    pub orbit_l1: f32,
    /// Orbital radius for signal nodes (dimension → signal distance).
    pub orbit_l2: f32,
    /// Base sphere radius for the root node.
    pub root_base_radius: f32,
    /// Base sphere radius for dimension nodes.
    pub dim_base_radius: f32,
    /// Base sphere radius for signal nodes.
    pub sig_base_radius: f32,
    /// Score-driven size multiplier applied to root and dimension radii.
    /// A composite of 80 with `score_scale=0.003` adds 0.24 to the base.
    pub score_scale: f32,
    /// Points-driven size multiplier applied to signal radii.
    /// A 40-point signal with `sig_points_scale=0.008` adds 0.32 to the base.
    pub sig_points_scale: f32,
    /// Signal-count boost applied to dimension node radii.
    /// A dimension with 5 signals at `dim_signal_scale=0.02` adds 0.10 to the base.
    pub dim_signal_scale: f32,
    /// Extra radius added to signal nodes that carry an actionable detail string
    /// (`MaturitySignal.detail.is_some()`), indicating a concrete finding.
    pub sig_detail_boost: f32,
}

impl Default for NodeSizeConfig {
    fn default() -> Self {
        Self {
            orbit_l1: 5.0,
            orbit_l2: 2.0,
            root_base_radius: 0.50,
            dim_base_radius: 0.28,
            sig_base_radius: 0.12,
            score_scale: 0.003,
            sig_points_scale: 0.008,
            dim_signal_scale: 0.02,
            sig_detail_boost: 0.08,
        }
    }
}

fn default_tier() -> u8 { 1 }

// ── Public types ─────────────────────────────────────────────────────────────

/// A node in the decision graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    /// Stable, deterministic identifier.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Node kind: `"root"`, `"dimension"`, or `"signal"`.
    pub kind: String,

    // ── Pre-computed 3D world-space position (distance-vector radial layout) ──
    /// World-space X coordinate.
    pub px: f32,
    /// World-space Y coordinate.
    pub py: f32,
    /// World-space Z coordinate.
    pub pz: f32,
    /// Pre-computed visual sphere radius derived from [`NodeSizeConfig`].
    pub radius: f32,

    // ── Semantic metadata (inspector + future analytics) ───────────────────
    /// Actual value earned: composite score for root, dimension score for
    /// dimension nodes, or points-earned (0 if failed) for signal nodes.
    pub score: f32,
    /// Dimension weight (0.0–1.0); 1.0 for root; parent dimension weight for
    /// signal nodes.
    pub weight: f32,
    /// Maximum points this signal can contribute; 0.0 for root/dimension nodes.
    pub max_points: f32,
    /// Number of direct child signals; 0 for root and signal nodes.
    pub signal_count: u32,

    // ── Pass/fail ────────────────────────────────────────────────────────────
    /// Whether the node passed / is healthy.
    pub passed: bool,
    /// Whether this node should be highlighted (failed signal or
    /// below-threshold dimension).
    pub highlight: bool,
    /// Whether this signal node carries an actionable `detail` string.
    /// Always `false` for root and dimension nodes.
    pub has_detail: bool,
    /// Maturity scan tier this node belongs to (1/2/3).
    /// Always `1` for root and dimension nodes.
    #[serde(default = "default_tier")]
    pub tier: u8,
}

/// A directed edge from parent to child.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Source node id.
    pub from: String,
    /// Target node id.
    pub to: String,
    /// Visual weight for edge rendering (0.0–1.0).
    /// Root → dimension edges use the dimension's weight.
    /// Dimension → signal edges use half the parent dimension's weight.
    pub weight: f32,
}

/// Full decision graph for one scan, including the layout configuration that
/// was used to compute node positions and radii.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// The [`NodeSizeConfig`] used to position and size the nodes.
    pub config: NodeSizeConfig,
}

// ── Node-ID helpers ──────────────────────────────────────────────────────────

const ROOT_ID: &str = "root";

fn dim_id(dim_label: &str) -> String {
    format!("dim:{}", dim_label)
}

fn sig_id(dim_label: &str, sig_name: &str) -> String {
    format!("sig:{}:{}", dim_label, sig_name)
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Score below which a dimension node is considered failing / highlighted.
const DIM_PASS_THRESHOLD: u8 = 50;

// ── Transformation ───────────────────────────────────────────────────────────

impl DecisionGraph {
    /// Build a [`DecisionGraph`] from a [`MaturityScore`] using default layout.
    pub fn from_maturity(score: &MaturityScore) -> Self {
        Self::from_maturity_full(score, None, NodeSizeConfig::default())
    }

    /// Build a [`DecisionGraph`] from a [`MaturityScore`] with a custom layout.
    /// Delegates to [`from_maturity_full`] with `risk_score = None`.
    pub fn from_maturity_with_config(score: &MaturityScore, config: NodeSizeConfig) -> Self {
        Self::from_maturity_full(score, None, config)
    }

    /// Build a [`DecisionGraph`] with full control over the layout config and an
    /// optional `risk_score` from the LLM final report.  When `risk_score` is
    /// `Some(n)` it replaces the composite score for the root node’s visual radius,
    /// making high-risk repositories visually prominent at first glance.
    #[instrument(skip(score, config), fields(
        composite = score.composite,
        dim_count  = score.dimensions.len(),
    ))]
    pub fn from_maturity_full(
        score: &MaturityScore,
        risk_score: Option<u8>,
        config: NodeSizeConfig,
    ) -> Self {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        let n_dims = score.dimensions.len();
        let root_score = risk_score.unwrap_or(score.composite) as f32;

        // ── Root node (world origin) ─────────────────────────────────────────
        nodes.push(GraphNode {
            id: ROOT_ID.to_string(),
            label: format!("Composite {}", score.composite),
            kind: "root".to_string(),
            px: 0.0,
            py: 0.0,
            pz: 0.0,
            radius: config.root_base_radius + root_score * config.score_scale,
            score: score.composite as f32,
            weight: 1.0,
            max_points: 0.0,
            signal_count: 0,
            passed: true,
            highlight: false,
            has_detail: false,
            tier: 1,
        });

        // ── Dimension and signal nodes ───────────────────────────────────────
        for (dim_idx, dim) in score.dimensions.iter().enumerate() {
            let dim_label = dim.dimension.label();
            let weight = dim.dimension.weight();
            let did = dim_id(dim_label);
            let dim_passed = dim.score >= DIM_PASS_THRESHOLD;
            let n_sigs = dim.signals.len();

            // Evenly distribute dimension nodes on a circle in the XZ plane.
            let dim_angle = TAU * (dim_idx as f32) / (n_dims.max(1) as f32);
            let dim_px = config.orbit_l1 * dim_angle.cos();
            let dim_pz = config.orbit_l1 * dim_angle.sin();

            nodes.push(GraphNode {
                id: did.clone(),
                label: dim_label.to_string(),
                kind: "dimension".to_string(),
                px: dim_px,
                py: 0.0,
                pz: dim_pz,
                radius: config.dim_base_radius
                    + dim.score as f32 * config.score_scale
                    + n_sigs as f32 * config.dim_signal_scale,
                score: dim.score as f32,
                weight,
                max_points: 0.0,
                signal_count: n_sigs as u32,
                passed: dim_passed,
                highlight: !dim_passed,
                has_detail: false,
                tier: 1,
            });

            edges.push(GraphEdge {
                from: ROOT_ID.to_string(),
                to: did.clone(),
                weight,
            });

            // Tangent basis vectors for the signal sub-circle.
            // The XZ radial direction to this dimension is (cos θ, 0, sin θ).
            // The tangential direction in XZ is (-sin θ, 0, cos θ).
            // The up direction is (0, 1, 0).
            // Signals are distributed on the circle spanned by tangent × up,
            // so each dimension's cluster fans out perpendicular to its radius.
            let tang_x = -dim_angle.sin();
            let tang_z = dim_angle.cos();

            for (sig_idx, sig) in dim.signals.iter().enumerate() {
                let sid = sig_id(dim_label, &sig.name);
                let has_detail = sig.detail.is_some();

                let sig_angle = TAU * (sig_idx as f32) / (n_sigs.max(1) as f32);
                // Expand orbit radius so signals don't overlap when a dimension
                // has many signals: require at least 0.4 units of arc per signal.
                let effective_orbit_l2 = f32::max(config.orbit_l2, n_sigs as f32 * 0.4);
                // cos component: along the tangent in XZ
                // sin component: along the vertical (Y)
                let sig_px = dim_px + effective_orbit_l2 * sig_angle.cos() * tang_x;
                let sig_py = effective_orbit_l2 * sig_angle.sin();
                let sig_pz = dim_pz + effective_orbit_l2 * sig_angle.cos() * tang_z;

                let sig_radius = config.sig_base_radius
                    + sig.points as f32 * config.sig_points_scale
                    + if has_detail {
                        config.sig_detail_boost
                    } else {
                        0.0
                    };

                nodes.push(GraphNode {
                    id: sid.clone(),
                    label: sig.description.clone(),
                    kind: "signal".to_string(),
                    px: sig_px,
                    py: sig_py,
                    pz: sig_pz,
                    radius: sig_radius,
                    score: if sig.passed { sig.points as f32 } else { 0.0 },
                    weight,
                    max_points: sig.points as f32,
                    signal_count: 0,
                    passed: sig.passed,
                    highlight: !sig.passed,
                    has_detail,
                    tier: sig.tier,
                });

                edges.push(GraphEdge {
                    from: did.clone(),
                    to: sid,
                    weight: weight * 0.5,
                });
            }
        }

        let highlight_count = nodes.iter().filter(|n| n.highlight).count();
        tracing::debug!(
            node_count = nodes.len(),
            edge_count = edges.len(),
            highlight_count,
            "DecisionGraph built"
        );

        Self {
            nodes,
            edges,
            config,
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::maturity::{
        DimensionScore, MaturityDimension, MaturityGrade, MaturityScore, MaturitySignal,
    };

    fn make_signal(name: &str, passed: bool, points: u8) -> MaturitySignal {
        MaturitySignal {
            name: name.to_string(),
            description: format!("{name} description"),
            passed,
            points,
            detail: if passed {
                None
            } else {
                Some(format!("{name} failed"))
            },
            tier: 1,
        }
    }

    fn minimal_score() -> MaturityScore {
        let dim_security = DimensionScore {
            dimension: MaturityDimension::Security,
            score: 75,
            signals: vec![
                make_signal("no_critical_cves", true, 40),
                make_signal("has_security_policy", false, 20),
            ],
        };
        let dim_governance = DimensionScore {
            dimension: MaturityDimension::ProjectGovernance,
            score: 30, // below threshold — should be highlighted
            signals: vec![
                make_signal("has_readme", true, 30),
                make_signal("has_license", false, 30),
            ],
        };
        MaturityScore {
            composite: 60,
            grade: MaturityGrade::Gold,
            dimensions: vec![dim_security, dim_governance],
        }
    }

    // ── Node ID determinism ──────────────────────────────────────────────────

    #[test]
    fn root_node_id_is_always_root() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        assert!(
            graph.nodes.iter().any(|n| n.id == "root"),
            "root node with id='root' must always be present"
        );
    }

    #[test]
    fn dimension_node_ids_are_deterministic() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        assert!(
            graph.nodes.iter().any(|n| n.id == "dim:Security"),
            "dimension node id must be 'dim:<label>'"
        );
        assert!(
            graph.nodes.iter().any(|n| n.id == "dim:Project Governance"),
            "dimension node id must be 'dim:Project Governance'"
        );
    }

    #[test]
    fn signal_node_ids_are_deterministic() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        assert!(
            graph
                .nodes
                .iter()
                .any(|n| n.id == "sig:Security:no_critical_cves"),
            "signal node id must be 'sig:<dim>:<name>'"
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|n| n.id == "sig:Security:has_security_policy"),
            "second signal in Security must be present"
        );
    }

    // ── Edge relationships ───────────────────────────────────────────────────

    #[test]
    fn root_has_edges_to_each_dimension() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let root_targets: Vec<&str> = graph
            .edges
            .iter()
            .filter(|e| e.from == "root")
            .map(|e| e.to.as_str())
            .collect();
        assert!(
            root_targets.contains(&"dim:Security"),
            "no root→dim:Security edge"
        );
        assert!(
            root_targets.contains(&"dim:Project Governance"),
            "no root→dim:Project Governance edge"
        );
    }

    #[test]
    fn dimension_has_edges_to_its_signals() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sec_targets: Vec<&str> = graph
            .edges
            .iter()
            .filter(|e| e.from == "dim:Security")
            .map(|e| e.to.as_str())
            .collect();
        assert!(
            sec_targets.contains(&"sig:Security:no_critical_cves"),
            "dim:Security must have edge to its signals"
        );
        assert!(
            sec_targets.contains(&"sig:Security:has_security_policy"),
            "dim:Security must have edge to second signal"
        );
    }

    #[test]
    fn total_edges_equals_dimensions_plus_all_signals() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        // 2 root→dim edges + 2 signals in Security + 2 signals in Governance = 6
        assert_eq!(
            graph.edges.len(),
            6,
            "expected 6 edges (2 root→dim + 4 dim→sig)"
        );
    }

    // ── Semantic metadata ────────────────────────────────────────────────────

    #[test]
    fn root_node_score_equals_composite() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let root = graph.nodes.iter().find(|n| n.id == "root").unwrap();
        assert_eq!(root.score, 60.0, "root.score must equal composite score");
    }

    #[test]
    fn dimension_node_score_equals_dimension_score() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sec = graph.nodes.iter().find(|n| n.id == "dim:Security").unwrap();
        assert_eq!(sec.score, 75.0, "dim.score must equal dimension score");
    }

    #[test]
    fn dimension_node_weight_equals_dimension_weight() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sec = graph.nodes.iter().find(|n| n.id == "dim:Security").unwrap();
        assert!(
            (sec.weight - MaturityDimension::Security.weight()).abs() < 0.001,
            "dim.weight must equal dimension weight (got {})",
            sec.weight
        );
    }

    #[test]
    fn passing_signal_score_and_max_points() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sig = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:no_critical_cves")
            .unwrap();
        assert_eq!(
            sig.score, 40.0,
            "passing signal score must equal its points"
        );
        assert_eq!(
            sig.max_points, 40.0,
            "signal max_points must equal its points"
        );
    }

    #[test]
    fn failing_signal_score_is_zero_max_points_preserved() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sig = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:has_security_policy")
            .unwrap();
        assert_eq!(sig.score, 0.0, "failing signal score must be 0");
        assert_eq!(
            sig.max_points, 20.0,
            "failing signal max_points must still equal points"
        );
    }

    // ── Highlight logic ──────────────────────────────────────────────────────

    #[test]
    fn failed_signals_are_highlighted() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sig = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:has_security_policy")
            .unwrap();
        assert!(sig.highlight, "failed signal must have highlight=true");
        assert!(!sig.passed, "failed signal must have passed=false");
    }

    #[test]
    fn passed_signals_are_not_highlighted() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sig = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:no_critical_cves")
            .unwrap();
        assert!(!sig.highlight, "passed signal must not be highlighted");
        assert!(sig.passed, "passed signal must have passed=true");
    }

    #[test]
    fn below_threshold_dimension_is_highlighted() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let gov = graph
            .nodes
            .iter()
            .find(|n| n.id == "dim:Project Governance")
            .unwrap();
        // score=30 < DIM_PASS_THRESHOLD=50
        assert!(gov.highlight, "low-score dimension must be highlighted");
        assert!(!gov.passed, "low-score dimension must have passed=false");
    }

    #[test]
    fn above_threshold_dimension_is_not_highlighted() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sec = graph.nodes.iter().find(|n| n.id == "dim:Security").unwrap();
        // score=75 > DIM_PASS_THRESHOLD=50
        assert!(!sec.highlight, "healthy dimension must not be highlighted");
        assert!(sec.passed, "healthy dimension must have passed=true");
    }

    // ── Structural completeness ──────────────────────────────────────────────

    #[test]
    fn node_count_matches_structure() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        // 1 root + 2 dims + 2 sigs in Security + 2 sigs in Governance = 7
        assert_eq!(graph.nodes.len(), 7, "expected 7 nodes");
    }

    #[test]
    fn all_node_ids_are_unique() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
        let mut dedup = ids.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(ids.len(), dedup.len(), "node IDs must be unique");
    }

    #[test]
    fn all_edges_reference_existing_nodes() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let ids: std::collections::HashSet<&str> =
            graph.nodes.iter().map(|n| n.id.as_str()).collect();
        for edge in &graph.edges {
            assert!(
                ids.contains(edge.from.as_str()),
                "edge.from '{}' is not a valid node",
                edge.from
            );
            assert!(
                ids.contains(edge.to.as_str()),
                "edge.to '{}' is not a valid node",
                edge.to
            );
        }
    }

    // ── Position (distance-vector layout) ───────────────────────────────────

    #[test]
    fn root_node_is_at_world_origin() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let root = graph.nodes.iter().find(|n| n.id == "root").unwrap();
        assert!(root.px.abs() < 1e-5, "root px must be 0");
        assert!(root.py.abs() < 1e-5, "root py must be 0");
        assert!(root.pz.abs() < 1e-5, "root pz must be 0");
    }

    #[test]
    fn dimension_nodes_are_at_orbit_l1_distance_from_origin() {
        let config = NodeSizeConfig::default();
        let graph = DecisionGraph::from_maturity(&minimal_score());
        for node in graph.nodes.iter().filter(|n| n.kind == "dimension") {
            let dist = (node.px * node.px + node.pz * node.pz).sqrt();
            assert!(
                (dist - config.orbit_l1).abs() < 1e-4,
                "dimension '{}' must be at orbit_l1 ({}) from origin, got {}",
                node.id,
                config.orbit_l1,
                dist
            );
        }
    }

    #[test]
    fn signal_nodes_are_at_orbit_l2_distance_from_parent_dimension() {
        let config = NodeSizeConfig::default();
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let node_map: std::collections::HashMap<&str, &GraphNode> =
            graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

        for edge in graph.edges.iter().filter(|e| {
            node_map
                .get(e.from.as_str())
                .map_or(false, |n| n.kind == "dimension")
        }) {
            let parent = node_map[edge.from.as_str()];
            let child = node_map[edge.to.as_str()];
            let dx = child.px - parent.px;
            let dy = child.py - parent.py;
            let dz = child.pz - parent.pz;
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            assert!(
                (dist - config.orbit_l2).abs() < 1e-4,
                "signal '{}' must be at orbit_l2 ({}) from parent '{}', got {}",
                child.id,
                config.orbit_l2,
                parent.id,
                dist
            );
        }
    }

    // ── Radius ───────────────────────────────────────────────────────────────

    #[test]
    fn root_radius_is_larger_than_base() {
        let config = NodeSizeConfig::default();
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let root = graph.nodes.iter().find(|n| n.id == "root").unwrap();
        assert!(
            root.radius > config.root_base_radius,
            "root radius ({}) must exceed base ({})",
            root.radius,
            config.root_base_radius
        );
    }

    #[test]
    fn higher_score_dimension_has_larger_radius() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sec = graph.nodes.iter().find(|n| n.id == "dim:Security").unwrap(); // 75
        let gov = graph
            .nodes
            .iter()
            .find(|n| n.id == "dim:Project Governance")
            .unwrap(); // 30
        assert!(
            sec.radius > gov.radius,
            "Security (score=75) must have larger radius than Governance (score=30)"
        );
    }

    #[test]
    fn higher_points_signal_has_larger_radius() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let high = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:no_critical_cves")
            .unwrap(); // 40 pts
        let low = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:has_security_policy")
            .unwrap(); // 20 pts
        assert!(
            high.radius > low.radius,
            "40-point signal must have larger radius than 20-point signal"
        );
    }

    // ── Config round-trip ────────────────────────────────────────────────────

    #[test]
    fn graph_preserves_config() {
        let mut config = NodeSizeConfig::default();
        config.orbit_l1 = 8.0; // non-default value
        let graph = DecisionGraph::from_maturity_with_config(&minimal_score(), config.clone());
        assert_eq!(
            graph.config.orbit_l1, 8.0,
            "graph must preserve the config it was built with"
        );
    }

    // ── Phase 3: has_detail ──────────────────────────────────────────────────

    #[test]
    fn failing_signal_has_detail_true() {
        // make_signal sets detail=Some(...) for failing signals
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sig = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:has_security_policy")
            .unwrap();
        assert!(!sig.passed, "signal must be failed");
        assert!(
            sig.has_detail,
            "failing signal with detail text must have has_detail=true"
        );
    }

    #[test]
    fn passing_signal_has_detail_false() {
        // make_signal sets detail=None for passing signals
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sig = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:no_critical_cves")
            .unwrap();
        assert!(sig.passed, "signal must be passing");
        assert!(
            !sig.has_detail,
            "passing signal with no detail must have has_detail=false"
        );
    }

    #[test]
    fn root_and_dim_nodes_never_have_detail() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        for node in graph.nodes.iter().filter(|n| n.kind != "signal") {
            assert!(
                !node.has_detail,
                "{} node '{}' must always have has_detail=false",
                node.kind, node.id
            );
        }
    }

    #[test]
    fn signal_with_detail_has_larger_radius_than_same_points_without() {
        // Construct two signals with equal points but one has detail
        let dim = DimensionScore {
            dimension: MaturityDimension::Security,
            score: 50,
            signals: vec![
                MaturitySignal {
                    name: "with_detail".to_string(),
                    description: "with detail".to_string(),
                    passed: true,
                    points: 20,
                    detail: Some("actionable finding".to_string()),
                    tier: 1,
                },
                MaturitySignal {
                    name: "without_detail".to_string(),
                    description: "without detail".to_string(),
                    passed: true,
                    points: 20,
                    detail: None,
                    tier: 1,
                },
            ],
        };
        let score = MaturityScore {
            composite: 50,
            grade: MaturityGrade::Gold,
            dimensions: vec![dim],
        };
        let graph = DecisionGraph::from_maturity(&score);
        let with_d = graph
            .nodes
            .iter()
            .find(|n| n.id.contains("with_detail"))
            .unwrap();
        let without_d = graph
            .nodes
            .iter()
            .find(|n| n.id.contains("without_detail"))
            .unwrap();
        assert!(
            with_d.radius > without_d.radius,
            "signal with detail ({}) must have larger radius than same-points signal without ({})",
            with_d.radius,
            without_d.radius
        );
    }

    // ── Phase 3: dim radius driven by signal_count ───────────────────────────

    #[test]
    fn dimension_with_more_signals_has_larger_radius_at_same_score() {
        let score_small = MaturityScore {
            composite: 60,
            grade: MaturityGrade::Gold,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 60,
                signals: vec![make_signal("a", true, 10)], // 1 signal
            }],
        };
        let score_large = MaturityScore {
            composite: 60,
            grade: MaturityGrade::Gold,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 60,
                signals: vec![
                    make_signal("a", true, 10),
                    make_signal("b", true, 10),
                    make_signal("c", true, 10),
                    make_signal("d", true, 10),
                ], // 4 signals
            }],
        };
        let r_small = DecisionGraph::from_maturity(&score_small)
            .nodes
            .iter()
            .find(|n| n.id == "dim:Security")
            .unwrap()
            .radius;
        let r_large = DecisionGraph::from_maturity(&score_large)
            .nodes
            .iter()
            .find(|n| n.id == "dim:Security")
            .unwrap()
            .radius;
        assert!(
            r_large > r_small,
            "dim with 4 signals ({}) must be larger than dim with 1 signal ({})",
            r_large,
            r_small
        );
    }

    // ── Phase 3: edge weights ────────────────────────────────────────────────

    #[test]
    fn root_dim_edges_carry_dimension_weight() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sec_weight = MaturityDimension::Security.weight();
        let edge = graph
            .edges
            .iter()
            .find(|e| e.from == "root" && e.to == "dim:Security")
            .unwrap();
        assert!(
            (edge.weight - sec_weight).abs() < 1e-5,
            "root→dim edge weight must equal dimension weight (expected {}, got {})",
            sec_weight,
            edge.weight
        );
    }

    #[test]
    fn dim_sig_edges_carry_half_dimension_weight() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sec_weight = MaturityDimension::Security.weight();
        let edge = graph
            .edges
            .iter()
            .find(|e| e.from == "dim:Security" && e.to.starts_with("sig:"))
            .unwrap();
        assert!(
            (edge.weight - sec_weight * 0.5).abs() < 1e-5,
            "dim→sig edge weight must be half the dimension weight (expected {}, got {})",
            sec_weight * 0.5,
            edge.weight
        );
    }

    // ── Phase 3: from_maturity_full with risk_score ──────────────────────────

    #[test]
    fn from_maturity_full_with_risk_score_changes_root_radius() {
        let score = minimal_score(); // composite = 60
        let config = NodeSizeConfig::default();

        let graph_no_risk = DecisionGraph::from_maturity_full(&score, None, config.clone());
        let graph_high_risk = DecisionGraph::from_maturity_full(&score, Some(95), config.clone());

        let r_no_risk = graph_no_risk
            .nodes
            .iter()
            .find(|n| n.id == "root")
            .unwrap()
            .radius;
        let r_high_risk = graph_high_risk
            .nodes
            .iter()
            .find(|n| n.id == "root")
            .unwrap()
            .radius;

        assert!(
            r_high_risk > r_no_risk,
            "risk_score=95 root ({}) must be larger than no-risk root ({})",
            r_high_risk,
            r_no_risk
        );
    }

    #[test]
    fn from_maturity_full_none_risk_score_matches_composite() {
        let score = minimal_score(); // composite = 60
        let config = NodeSizeConfig::default();

        let graph_full = DecisionGraph::from_maturity_full(&score, None, config.clone());
        let graph_default = DecisionGraph::from_maturity(&score);

        let r_full = graph_full
            .nodes
            .iter()
            .find(|n| n.id == "root")
            .unwrap()
            .radius;
        let r_default = graph_default
            .nodes
            .iter()
            .find(|n| n.id == "root")
            .unwrap()
            .radius;

        assert!(
            (r_full - r_default).abs() < 1e-5,
            "from_maturity_full(None) root radius ({}) must equal from_maturity ({})",
            r_full,
            r_default
        );
    }

    // ── Phase 10: dynamic orbit_l2 scaling ──────────────────────────────────

    /// A dimension with 10 signals must push each signal out to 4.0 (= 10 × 0.4),
    /// which is larger than the default orbit_l2 of 2.0.
    #[test]
    fn large_dim_expands_orbit_to_fit_signals() {
        let many_signals: Vec<MaturitySignal> = (0..10)
            .map(|i| make_signal(&format!("sig{i}"), true, 5))
            .collect();
        let score = MaturityScore {
            composite: 50,
            grade: MaturityGrade::Gold,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 50,
                signals: many_signals,
            }],
        };
        let config = NodeSizeConfig::default(); // orbit_l2 = 2.0
        let expected_orbit = f32::max(config.orbit_l2, 10.0 * 0.4); // = 4.0
        let graph = DecisionGraph::from_maturity_with_config(&score, config);
        let node_map: std::collections::HashMap<&str, &GraphNode> =
            graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let parent = node_map["dim:Security"];
        for edge in graph.edges.iter().filter(|e| e.from == "dim:Security") {
            let child = node_map[edge.to.as_str()];
            let dx = child.px - parent.px;
            let dy = child.py - parent.py;
            let dz = child.pz - parent.pz;
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            assert!(
                (dist - expected_orbit).abs() < 1e-4,
                "signal '{}' must be at {expected_orbit} from parent (10-signal dim), got {dist}",
                child.id,
            );
        }
    }

    /// A dimension with 2 signals does NOT expand: 2 × 0.4 = 0.8 < default 2.0.
    #[test]
    fn small_dim_keeps_configured_orbit() {
        let score = minimal_score(); // each dim has 2 signals
        let config = NodeSizeConfig::default(); // orbit_l2 = 2.0
        let graph = DecisionGraph::from_maturity_with_config(&score, config.clone());
        let node_map: std::collections::HashMap<&str, &GraphNode> =
            graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let parent = node_map["dim:Security"];
        for edge in graph.edges.iter().filter(|e| e.from == "dim:Security") {
            let child = node_map[edge.to.as_str()];
            let dx = child.px - parent.px;
            let dy = child.py - parent.py;
            let dz = child.pz - parent.pz;
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            assert!(
                (dist - config.orbit_l2).abs() < 1e-4,
                "signal '{}' must stay at orbit_l2={} (2 signals don't expand), got {}",
                child.id,
                config.orbit_l2,
                dist
            );
        }
    }

    /// When orbit_l2 is explicitly set large, it remains the floor even for tiny dims.
    #[test]
    fn custom_large_orbit_respected_as_floor() {
        let score = MaturityScore {
            composite: 50,
            grade: MaturityGrade::Gold,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 50,
                signals: vec![make_signal("only_one", true, 10)], // 1 signal, 1*0.4 = 0.4
            }],
        };
        let mut config = NodeSizeConfig::default();
        config.orbit_l2 = 5.0; // large explicit value
        let graph = DecisionGraph::from_maturity_with_config(&score, config.clone());
        let node_map: std::collections::HashMap<&str, &GraphNode> =
            graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let parent = node_map["dim:Security"];
        let edge = graph
            .edges
            .iter()
            .find(|e| e.from == "dim:Security")
            .unwrap();
        let child = node_map[edge.to.as_str()];
        let dx = child.px - parent.px;
        let dy = child.py - parent.py;
        let dz = child.pz - parent.pz;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        assert!(
            (dist - 5.0).abs() < 1e-4,
            "single-signal dim must use configured orbit_l2=5.0, got {dist}"
        );
    }

    // ── Phase 6: NodeSizeConfig serde round-trip ─────────────────────────────

    #[test]
    fn node_size_config_serde_round_trip() {
        let config = NodeSizeConfig::default();
        let json = serde_json::to_string(&config).expect("serialize NodeSizeConfig");
        let back: NodeSizeConfig = serde_json::from_str(&json).expect("deserialize NodeSizeConfig");
        assert_eq!(back, config, "deserialized config must equal the original");
        // Verify individual fields explicitly so a missing field is easy to diagnose.
        assert_eq!(back.orbit_l1, config.orbit_l1);
        assert_eq!(back.orbit_l2, config.orbit_l2);
        assert_eq!(back.root_base_radius, config.root_base_radius);
        assert_eq!(back.dim_base_radius, config.dim_base_radius);
        assert_eq!(back.sig_base_radius, config.sig_base_radius);
        assert_eq!(back.score_scale, config.score_scale);
        assert_eq!(back.sig_points_scale, config.sig_points_scale);
        assert_eq!(back.dim_signal_scale, config.dim_signal_scale);
        assert_eq!(back.sig_detail_boost, config.sig_detail_boost);
    }
}
