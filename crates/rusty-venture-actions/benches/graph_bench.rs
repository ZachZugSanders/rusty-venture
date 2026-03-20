//! Benchmarks for decision-graph transformation.
//!
//! Run with:
//!   cargo bench -p rusty-venture-actions --bench graph_bench

use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use rusty_venture_actions::repo::maturity::{
    DimensionScore, MaturityDimension, MaturityGrade, MaturityScore, MaturitySignal,
};
use rusty_venture_actions::DecisionGraph;

// ── Fixture helpers ───────────────────────────────────────────────────────────

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
    }
}

fn make_dim(dim: MaturityDimension, score: u8, signal_count: usize) -> DimensionScore {
    let signals = (0..signal_count)
        .map(|i| make_signal(&format!("signal_{i}"), i % 3 != 0, (10 + i as u8).min(50)))
        .collect();
    DimensionScore {
        dimension: dim,
        score,
        signals,
    }
}

/// Minimal score: 1 dimension, 2 signals.
fn minimal_score() -> MaturityScore {
    MaturityScore {
        composite: 60,
        grade: MaturityGrade::Gold,
        dimensions: vec![make_dim(MaturityDimension::Security, 70, 2)],
    }
}

/// Typical score: all 6 dimensions, each with 4 signals.
fn typical_score() -> MaturityScore {
    MaturityScore {
        composite: 72,
        grade: MaturityGrade::Gold,
        dimensions: vec![
            make_dim(MaturityDimension::Security, 80, 4),
            make_dim(MaturityDimension::DependencyHealth, 75, 4),
            make_dim(MaturityDimension::BuildAndCi, 65, 4),
            make_dim(MaturityDimension::CodeOrganization, 70, 4),
            make_dim(MaturityDimension::ProjectGovernance, 60, 4),
            make_dim(MaturityDimension::TestingAndQuality, 55, 4),
        ],
    }
}

/// Large-scale score: all 6 dimensions, 20 signals each (120 signal nodes total).
fn large_score() -> MaturityScore {
    MaturityScore {
        composite: 68,
        grade: MaturityGrade::Gold,
        dimensions: vec![
            make_dim(MaturityDimension::Security, 80, 20),
            make_dim(MaturityDimension::DependencyHealth, 75, 20),
            make_dim(MaturityDimension::BuildAndCi, 65, 20),
            make_dim(MaturityDimension::CodeOrganization, 70, 20),
            make_dim(MaturityDimension::ProjectGovernance, 60, 20),
            make_dim(MaturityDimension::TestingAndQuality, 55, 20),
        ],
    }
}

// ── Benchmarks ────────────────────────────────────────────────────────────────

fn bench_from_maturity(c: &mut Criterion) {
    let mut group = c.benchmark_group("DecisionGraph::from_maturity");

    let cases = [
        ("minimal (1 dim / 2 sigs)", minimal_score()),
        ("typical (6 dims / 4 sigs each)", typical_score()),
        ("large (6 dims / 20 sigs each)", large_score()),
    ];

    for (label, score) in &cases {
        group.bench_with_input(BenchmarkId::new("scale", label), score, |b, s| {
            b.iter(|| DecisionGraph::from_maturity(black_box(s)))
        });
    }

    group.finish();
}

fn bench_json_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("DecisionGraph JSON round-trip");

    let score = typical_score();
    group.bench_function("typical (6 dims / 4 sigs each)", |b| {
        b.iter_batched(
            || DecisionGraph::from_maturity(black_box(&score)),
            |graph| {
                let json = serde_json::to_string(black_box(&graph)).unwrap();
                let _: DecisionGraph = serde_json::from_str(black_box(&json)).unwrap();
            },
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

criterion_group!(benches, bench_from_maturity, bench_json_roundtrip);
criterion_main!(benches);
