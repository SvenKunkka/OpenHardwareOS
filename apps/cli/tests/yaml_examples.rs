//! The example rules shipped in `examples/rules` must stay loadable.
//!
//! They are the first thing a new user copies into their config directory, so a
//! schema change that breaks them is a real regression, not a documentation
//! nit. This test parses every example, validates it structurally, and checks
//! that the engine accepts it against simulated hardware.

use ohm_automation::Rule;

#[test]
fn every_shipped_example_rule_parses_and_validates() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rules");
    let entries = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", directory.display()));

    let mut checked = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "yaml") {
            continue;
        }
        let raw = std::fs::read_to_string(&path).expect("readable");
        let rule: Rule = serde_yaml_ng::from_str(&raw)
            .unwrap_or_else(|error| panic!("{} does not parse: {error}", path.display()));
        rule.validate()
            .unwrap_or_else(|error| panic!("{} is structurally invalid: {error}", path.display()));
        assert!(!rule.name.is_empty(), "{}", path.display());
        assert!(
            rule.curve.len() >= 2,
            "{}: a curve needs at least two points",
            path.display()
        );
        assert!(
            raw.to_ascii_lowercase().contains("replace"),
            "{}: examples must tell the user to replace the device ids",
            path.display()
        );
        checked += 1;
    }
    assert!(
        checked >= 3,
        "expected the shipped examples, found {checked}"
    );
}

#[tokio::test]
async fn an_example_rule_is_accepted_by_the_engine_on_simulated_hardware() {
    use ohm_adapter_api::HardwareAdapter;
    use ohm_adapters::{AdapterOptions, build_adapters};
    use ohm_automation::{AutomationEngine, RuleStore};
    use ohm_core::ConfigPaths;
    use ohm_runtime::Runtime;
    use std::sync::Arc;

    let temp = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::from_root(temp.path());
    let adapters: Vec<Arc<dyn HardwareAdapter>> = build_adapters(&AdapterOptions::simulated_only());
    let runtime = Runtime::new(paths.clone(), ohm_runtime::Settings::default(), adapters).unwrap();
    runtime.start().await.unwrap();
    let engine = AutomationEngine::new(runtime.clone(), RuleStore::from_paths(&paths));

    let raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/rules/gpu-cooling.yaml"),
    )
    .unwrap();
    let rule: Rule = serde_yaml_ng::from_str(&raw).unwrap();
    let check = engine.check_rule(&rule);
    assert!(check.is_ok(), "errors: {:?}", check.errors);
    engine.save_rule(rule).unwrap();
    assert_eq!(engine.rule_count(), 1);

    runtime.shutdown().await.unwrap();
}
