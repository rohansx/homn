//! Criterion benchmarks for the policy engine (T087 — closes R-004).
//!
//! Measures three scenarios against the shipped `policies/example.rhai` ruleset (~45 rules,
//! representative of what a real user would write):
//!
//! - **early_deny**: a Bash call that matches the first deny rule. Best-case for the deny path.
//! - **mid_allow**:  a Read call that matches a mid-list allow rule. Typical case.
//! - **late_allow**: a `git push origin feat/foo` that matches near the end of the allows.
//! - **worst_no_match**: a call no rule matches. Forces the engine to evaluate every rule
//!   before falling through to the default `Ask`. Worst-case latency.
//!
//! The spec commits to ≤200ms p95 across all rules per call. We target one or two orders of
//! magnitude better; the bench produces a target/criterion HTML report you can browse.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use homn_policy::{Engine, EvalRequest, RuleSet};

const EXAMPLE_POLICY: &str = include_str!("../../../policies/example.rhai");

fn base_request() -> EvalRequest {
    EvalRequest {
        home: "/home/rsx".into(),
        cwd: "/home/rsx/dev/cloakpipe".into(),
        ..Default::default()
    }
}

fn parse_example_ruleset(engine: &Engine) -> RuleSet {
    RuleSet::parse(engine, EXAMPLE_POLICY, "example.rhai").expect("example.rhai parses")
}

fn bench_eval(c: &mut Criterion) {
    let engine = Engine::new();
    let rules = parse_example_ruleset(&engine);

    let early_deny = EvalRequest {
        tool: "Bash".into(),
        cmd: "rm -rf /home/rsx/scratch".into(),
        ..base_request()
    };

    let mid_allow = EvalRequest {
        tool: "Read".into(),
        path: "/home/rsx/foo.txt".into(),
        ..base_request()
    };

    let late_allow = EvalRequest {
        tool: "Bash".into(),
        cmd: "git push origin feat/some-feature".into(),
        ..base_request()
    };

    let worst_no_match = EvalRequest {
        tool: "WebFetch".into(),
        url: "https://example.com/some/path".into(),
        ..base_request()
    };

    let mut group = c.benchmark_group("rule_eval");
    // Criterion default sample_size is 100; bump to get tighter percentile estimates.
    group.sample_size(200);

    group.bench_function("early_deny", |b| {
        b.iter(|| engine.eval(black_box(&rules), black_box(&early_deny)))
    });

    group.bench_function("mid_allow", |b| {
        b.iter(|| engine.eval(black_box(&rules), black_box(&mid_allow)))
    });

    group.bench_function("late_allow", |b| {
        b.iter(|| engine.eval(black_box(&rules), black_box(&late_allow)))
    });

    group.bench_function("worst_no_match", |b| {
        b.iter(|| engine.eval(black_box(&rules), black_box(&worst_no_match)))
    });

    group.finish();
}

fn bench_parse(c: &mut Criterion) {
    let engine = Engine::new();
    c.bench_function("parse_example_ruleset", |b| {
        b.iter(|| RuleSet::parse(&engine, black_box(EXAMPLE_POLICY), "example.rhai").unwrap())
    });
}

criterion_group!(benches, bench_eval, bench_parse);
criterion_main!(benches);
