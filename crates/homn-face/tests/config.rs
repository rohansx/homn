//! Regression guard: tauri.conf.json must parse and carry every window key the
//! face spike depends on (transparent, alwaysOnTop, decorations, skipTaskbar, etc.).
//! If a future edit drops one of these keys silently, this test fails on the next CI run.

use serde_json::Value;

#[test]
fn tauri_conf_json_parses_with_expected_window_keys() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
    let raw = std::fs::read_to_string(path).expect("tauri.conf.json is readable");
    let v: Value = serde_json::from_str(&raw).expect("tauri.conf.json is valid JSON");

    let win = &v["app"]["windows"][0];
    for key in [
        "title",
        "width",
        "height",
        "transparent",
        "alwaysOnTop",
        "decorations",
        "resizable",
        "skipTaskbar",
    ] {
        assert!(
            !win[key].is_null(),
            "tauri.conf.json window[0] is missing the required key `{key}`"
        );
    }

    assert_eq!(
        win["transparent"],
        Value::Bool(true),
        "transparent must be true"
    );
    assert_eq!(
        win["alwaysOnTop"],
        Value::Bool(true),
        "alwaysOnTop must be true"
    );
    assert_eq!(
        win["decorations"],
        Value::Bool(false),
        "decorations must be false"
    );
    assert_eq!(win["width"], Value::from(200), "width must be 200");
    assert_eq!(win["height"], Value::from(120), "height must be 120");
    assert_eq!(
        win["resizable"],
        Value::Bool(false),
        "resizable must be false"
    );
    assert_eq!(
        win["skipTaskbar"],
        Value::Bool(true),
        "skipTaskbar must be true"
    );
    assert_eq!(win["title"], Value::from("homn"), "title must be \"homn\"");
}
