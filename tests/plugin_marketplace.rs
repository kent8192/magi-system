use std::path::Path;

use serde_json::Value;

#[test]
fn codex_marketplace_exposes_magi_plugin() {
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".agents/plugins/marketplace.json");
    let manifest =
        std::fs::read_to_string(&manifest_path).expect("Codex marketplace manifest should exist");
    let manifest: Value =
        serde_json::from_str(&manifest).expect("Codex marketplace manifest should be valid JSON");

    assert_eq!(manifest["name"], "magi");
    assert_eq!(manifest["plugins"][0]["name"], "magi");
    assert_eq!(manifest["plugins"][0]["source"]["source"], "local");
    assert_eq!(manifest["plugins"][0]["source"]["path"], "./plugins/magi");

    let plugin_manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join(".codex-plugin/plugin.json");
    assert!(
        plugin_manifest.exists(),
        "magi Codex plugin manifest should exist at .codex-plugin/plugin.json"
    );

    let marketplace_plugin_manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/magi/.codex-plugin/plugin.json");
    assert!(
        marketplace_plugin_manifest.exists(),
        "magi marketplace plugin should include a Codex plugin manifest"
    );
    assert_eq!(
        std::fs::read_to_string(plugin_manifest).expect("read root Codex plugin manifest"),
        std::fs::read_to_string(marketplace_plugin_manifest)
            .expect("read marketplace Codex plugin manifest")
    );

    let root_skill =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".codex-plugin/skills/magi/SKILL.md");
    let marketplace_skill = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("plugins/magi/.codex-plugin/skills/magi/SKILL.md");
    assert_eq!(
        std::fs::read_to_string(root_skill).expect("read root Codex skill"),
        std::fs::read_to_string(marketplace_skill).expect("read marketplace Codex skill")
    );
}

#[test]
fn installer_defaults_plugin_marketplace_to_local_checkout() {
    let installer = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let installer = std::fs::read_to_string(installer).expect("read installer");

    assert!(
        installer.contains(r#"MAGI_PLUGIN_REPO="${MAGI_PLUGIN_REPO:-$SCRIPT_DIR}""#),
        "installer should default plugin marketplace registration to the local checkout"
    );
}
