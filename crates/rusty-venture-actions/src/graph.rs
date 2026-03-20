//! Decision-graph transformation.
//!
//! Transforms a [`MaturityScore`] into a structured [`DecisionGraph`] payload
//! suitable for the 3D explorer.  The graph has three tiers of nodes:
//!
//! * **root** — one node representing the composite score.
//! * **dimension** — one node per maturity dimension.
//! * **signal** — one node per maturity signal inside each dimension.
//!
//! Node IDs are deterministic, derived from the tier and name, so the same
//! repository scan always produces the same graph structure.

use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::repo::maturity::MaturityScore;

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
    /// X-axis — dimension weight (0.0–1.0); 0.0 for root.
    pub x: f32,
    /// Y-axis — score/composite (0–100).
    pub y: f32,
    /// Z-axis — max points for a signal; 0 for root/dimension.
    pub z: f32,
    /// Whether the node passed / is healthy.
    pub passed: bool,
    /// Whether this node should be highlighted (failed signal or below-threshold dimension).
    pub highlight: bool,
}

/// A directed edge from parent to child.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Source node id.
    pub from: String,
    /// Target node id.
    pub to: String,
}

/// Full decision graph for one scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

// ── Node-ID helpers ──────────────────────────────────────────────────────────

const ROOT_ID: &str = "root";

fn dim_id(dim_label: &str) -> String {
    format!("dim:{}", dim_label)
}

fn sig_id(dim_label: &str, sig_name: &str) -> String {
    format!("sig:{}:{}", dim_label, sig_name)
}

// ── Transformation ───────────────────────────────────────────────────────────

/// Score below which a dimension node is considered failing / highlighted.
const DIM_PASS_THRESHOLD: u8 = 50;

impl DecisionGraph {
    /// Build a [`DecisionGraph`] from a [`MaturityScore`].
    #[instrument(skip(score), fields(
        composite = score.composite,
        dim_count = score.dimensions.len(),
    ))]
    pub fn from_maturity(score: &MaturityScore) -> Self {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        // Root node
        nodes.push(GraphNode {
            id: ROOT_ID.to_string(),
            label: format!("Composite {}", score.composite),
            kind: "root".to_string(),
            x: 0.0,
            y: score.composite as f32,
            z: 0.0,
            passed: true,
            highlight: false,
        });

        for dim in &score.dimensions {
            let dim_label = dim.dimension.label();
            let weight = dim.dimension.weight();
            let did = dim_id(dim_label);
            let dim_passed = dim.score >= DIM_PASS_THRESHOLD;

            // Dimension node
            nodes.push(GraphNode {
                id: did.clone(),
                label: dim_label.to_string(),
                kind: "dimension".to_string(),
                x: weight,
                y: dim.score as f32,
                z: 0.0,
                passed: dim_passed,
                highlight: !dim_passed,
            });

            // Edge: root → dimension
            edges.push(GraphEdge {
                from: ROOT_ID.to_string(),
                to: did.clone(),
            });

            for sig in &dim.signals {
                let sid = sig_id(dim_label, &sig.name);

                nodes.push(GraphNode {
                    id: sid.clone(),
                    label: sig.description.clone(),
                    kind: "signal".to_string(),
                    x: weight,
                    y: if sig.passed { sig.points as f32 } else { 0.0 },
                    z: sig.points as f32,
                    passed: sig.passed,
                    highlight: !sig.passed,
                });

                // Edge: dimension → signal
                edges.push(GraphEdge {
                    from: did.clone(),
                    to: sid,
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

        Self { nodes, edges }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::maturity::{DimensionScore, MaturityDimension, MaturityGrade, MaturityScore, MaturitySignal};

    fn make_signal(name: &str, passed: bool, points: u8) -> MaturitySignal {
        MaturitySignal {
            name: name.to_string(),
            description: format!("{name} description"),
            passed,
            points,
            detail: if passed { None } else { Some(format!("{name} failed")) },
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
            graph.nodes.iter().any(|n| n.id == "sig:Security:no_critical_cves"),
            "signal node id must be 'sig:<dim>:<name>'"
        );
        assert!(
            graph.nodes.iter().any(|n| n.id == "sig:Security:has_security_policy"),
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
        assert!(root_targets.contains(&"dim:Security"), "no root→dim:Security edge");
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
        assert_eq!(graph.edges.len(), 6, "expected 6 edges (2 root→dim + 4 dim→sig)");
    }

    // ── Axis values ─────────────────────────────────────────────────────────

    #[test]
    fn root_node_y_axis_equals_composite_score() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let root = graph.nodes.iter().find(|n| n.id == "root").unwrap();
        assert_eq!(root.y, 60.0, "root y must equal composite score");
    }

    #[test]
    fn dimension_node_y_axis_equals_dimension_score() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sec = graph.nodes.iter().find(|n| n.id == "dim:Security").unwrap();
        assert_eq!(sec.y, 75.0, "dim y must equal dimension score");
    }

    #[test]
    fn dimension_node_x_axis_equals_weight() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sec = graph.nodes.iter().find(|n| n.id == "dim:Security").unwrap();
        assert!(
            (sec.x - MaturityDimension::Security.weight()).abs() < 0.001,
            "dim x must equal dimension weight (got {})",
            sec.x
        );
    }

    #[test]
    fn passing_signal_y_axis_equals_points_value() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sig = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:no_critical_cves")
            .unwrap();
        assert_eq!(sig.y, 40.0, "passing signal y must equal points");
        assert_eq!(sig.z, 40.0, "signal z must equal max points");
    }

    #[test]
    fn failing_signal_y_axis_is_zero() {
        let graph = DecisionGraph::from_maturity(&minimal_score());
        let sig = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:has_security_policy")
            .unwrap();
        assert_eq!(sig.y, 0.0, "failing signal y must be 0");
        assert_eq!(sig.z, 20.0, "failing signal z must still equal max points");
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
            assert!(ids.contains(edge.from.as_str()), "edge.from '{}' is not a valid node", edge.from);
            assert!(ids.contains(edge.to.as_str()), "edge.to '{}' is not a valid node", edge.to);
        }
    }

    // ── Round-trip serialisation ─────────────────────────────────────────────

    #[test]
    fn decision_graph_round_trips_through_json() {
        let original = DecisionGraph::from_maturity(&minimal_score());
        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: DecisionGraph = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, decoded);
    }

    // ── Edge-case regression tests ───────────────────────────────────────────

    #[test]
    fn empty_dimensions_yields_only_root_node() {
        let score = MaturityScore {
            composite: 50,
            grade: MaturityGrade::Silver,
            dimensions: vec![],
        };
        let graph = DecisionGraph::from_maturity(&score);
        assert_eq!(graph.nodes.len(), 1, "only root node when no dimensions");
        assert_eq!(graph.edges.len(), 0, "no edges when no dimensions");
        assert_eq!(graph.nodes[0].id, "root");
    }

    #[test]
    fn dimension_with_no_signals_produces_root_dim_and_single_edge() {
        let score = MaturityScore {
            composite: 70,
            grade: MaturityGrade::Gold,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 70,
                signals: vec![],
            }],
        };
        let graph = DecisionGraph::from_maturity(&score);
        // 1 root + 1 dim = 2 nodes; 1 root→dim edge
        assert_eq!(graph.nodes.len(), 2, "root + dimension");
        assert_eq!(graph.edges.len(), 1, "one edge root→dim");
        assert_eq!(graph.edges[0].from, "root");
        assert_eq!(graph.edges[0].to, "dim:Security");
    }

    #[test]
    fn composite_0_sets_root_y_to_zero() {
        let score = MaturityScore {
            composite: 0,
            grade: MaturityGrade::Bronze,
            dimensions: vec![],
        };
        let graph = DecisionGraph::from_maturity(&score);
        assert_eq!(graph.nodes[0].y, 0.0, "composite 0 → root y = 0");
    }

    #[test]
    fn composite_100_sets_root_y_to_100() {
        let score = MaturityScore {
            composite: 100,
            grade: MaturityGrade::Diamond,
            dimensions: vec![],
        };
        let graph = DecisionGraph::from_maturity(&score);
        assert_eq!(graph.nodes[0].y, 100.0, "composite 100 → root y = 100");
    }

    #[test]
    fn dim_score_at_threshold_50_is_not_highlighted() {
        let score = MaturityScore {
            composite: 50,
            grade: MaturityGrade::Silver,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 50, // exactly at threshold — should pass
                signals: vec![],
            }],
        };
        let graph = DecisionGraph::from_maturity(&score);
        let dim = graph.nodes.iter().find(|n| n.id == "dim:Security").unwrap();
        assert!(!dim.highlight, "score=50 is at threshold, must NOT be highlighted");
        assert!(dim.passed, "score=50 must be passed=true");
    }

    #[test]
    fn dim_score_49_is_highlighted() {
        let score = MaturityScore {
            composite: 49,
            grade: MaturityGrade::Bronze,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 49, // just below threshold
                signals: vec![],
            }],
        };
        let graph = DecisionGraph::from_maturity(&score);
        let dim = graph.nodes.iter().find(|n| n.id == "dim:Security").unwrap();
        assert!(dim.highlight, "score=49 is below threshold, must be highlighted");
        assert!(!dim.passed, "score=49 must be passed=false");
    }

    #[test]
    fn signal_with_zero_points_has_z_axis_zero() {
        let score = MaturityScore {
            composite: 60,
            grade: MaturityGrade::Gold,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 60,
                signals: vec![MaturitySignal {
                    name: "zero_point_check".to_string(),
                    description: "zero pts".to_string(),
                    passed: true,
                    points: 0,
                    detail: None,
                }],
            }],
        };
        let graph = DecisionGraph::from_maturity(&score);
        let sig = graph
            .nodes
            .iter()
            .find(|n| n.id == "sig:Security:zero_point_check")
            .unwrap();
        assert_eq!(sig.z, 0.0, "zero-point signal must have z=0");
        assert_eq!(sig.y, 0.0, "passing zero-point signal must have y=0");
    }

    /// Smoke test covering a realistic full-scale score with all 6 dimensions.
    #[test]
    fn full_6_dimension_score_has_correct_node_and_edge_counts() {
        use MaturityDimension::*;
        let dims = [
            Security,
            DependencyHealth,
            BuildAndCi,
            CodeOrganization,
            ProjectGovernance,
            TestingAndQuality,
        ];
        let mut dimensions = vec![];
        for (i, dim) in dims.iter().enumerate() {
            dimensions.push(DimensionScore {
                dimension: *dim,
                score: 60 + i as u8 * 5,
                signals: vec![
                    make_signal("signal_a", true, 20),
                    make_signal("signal_b", false, 15),
                    make_signal("signal_c", true, 10),
                ],
            });
        }
        let score = MaturityScore {
            composite: 72,
            grade: MaturityGrade::Gold,
            dimensions,
        };
        let graph = DecisionGraph::from_maturity(&score);
        // 1 root + 6 dims + 6×3 signals = 25 nodes
        assert_eq!(graph.nodes.len(), 25, "1 + 6 + 18 = 25 nodes");
        // 6 root→dim + 18 dim→sig = 24 edges
        assert_eq!(graph.edges.len(), 24, "6 + 18 = 24 edges");
    }

    #[test]
    fn all_passing_score_has_no_highlights() {
        let score = MaturityScore {
            composite: 100,
            grade: MaturityGrade::Diamond,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 100,
                signals: vec![
                    make_signal("s1", true, 50),
                    make_signal("s2", true, 50),
                ],
            }],
        };
        let graph = DecisionGraph::from_maturity(&score);
        assert!(
            graph.nodes.iter().all(|n| !n.highlight),
            "fully-passing score must have zero highlighted nodes"
        );
    }

    #[test]
    fn all_failing_signals_all_highlighted() {
        let score = MaturityScore {
            composite: 10,
            grade: MaturityGrade::Bronze,
            dimensions: vec![DimensionScore {
                dimension: MaturityDimension::Security,
                score: 10, // below threshold
                signals: vec![
                    make_signal("s1", false, 40),
                    make_signal("s2", false, 40),
                ],
            }],
        };
        let graph = DecisionGraph::from_maturity(&score);
        // dim + both signals must be highlighted; root is never highlighted
        let non_root_highlighted = graph
            .nodes
            .iter()
            .filter(|n| n.id != "root" && n.highlight)
            .count();
        assert_eq!(non_root_highlighted, 3, "dim + 2 signals must all be highlighted");
    }
}
