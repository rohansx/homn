//! The shipped `eval/questions/TEMPLATE.toml` must parse and be structurally coherent.
//!
//! It intentionally does NOT have 10/10/10 filled in (it's a template with a few example blocks),
//! so we assert it parses, its meta loads, and every present question has an id + kind — the shape
//! a real set is authored into. Balance is enforced by `validate(true)` at run time (unit-tested in
//! `schema.rs`).

use homn_eval::QuestionSet;

#[test]
fn shipped_template_parses() {
    // tests/ runs with CWD = crate root (crates/homn-eval); the template is at the repo root.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../eval/questions/TEMPLATE.toml"
    );
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));

    let set = QuestionSet::from_toml_str(&text).expect("TEMPLATE.toml must parse");
    assert!(!set.meta.captured_week.is_empty());
    assert!(
        !set.questions.is_empty(),
        "template should show example question blocks"
    );
    for q in &set.questions {
        assert!(!q.id.is_empty(), "every question needs an id");
    }
}
