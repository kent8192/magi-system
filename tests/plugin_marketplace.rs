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
fn codex_plugin_declares_session_agent_hooks() {
    let root_manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".codex-plugin/plugin.json");
    let root_manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(root_manifest_path).expect("read root Codex plugin manifest"),
    )
    .expect("root Codex plugin manifest should be valid JSON");
    assert_eq!(root_manifest["hooks"], "./hooks/hooks.json");

    let marketplace_manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/magi/.codex-plugin/plugin.json");
    let marketplace_manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(marketplace_manifest_path)
            .expect("read marketplace Codex plugin manifest"),
    )
    .expect("marketplace Codex plugin manifest should be valid JSON");
    assert_eq!(marketplace_manifest["hooks"], "./hooks/hooks.json");

    let root_hooks = Path::new(env!("CARGO_MANIFEST_DIR")).join(".codex-plugin/hooks/hooks.json");
    let marketplace_hooks =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/magi/.codex-plugin/hooks/hooks.json");
    let hooks: Value = serde_json::from_str(
        &std::fs::read_to_string(&root_hooks).expect("read root Codex hooks config"),
    )
    .expect("root Codex hooks config should be valid JSON");
    assert!(hooks["hooks"]["SessionStart"].is_array());
    assert!(hooks["hooks"]["SessionEnd"].is_array());
    assert_eq!(
        std::fs::read_to_string(root_hooks).expect("read root Codex hooks config"),
        std::fs::read_to_string(marketplace_hooks).expect("read marketplace Codex hooks config")
    );

    for hook in ["magi-codex-session-start.sh", "magi-codex-session-end.sh"] {
        let root_hook = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".codex-plugin/hooks")
            .join(hook);
        let marketplace_hook = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("plugins/magi/.codex-plugin/hooks")
            .join(hook);
        assert!(
            root_hook.exists(),
            "{hook} should exist in root Codex plugin"
        );
        assert!(
            marketplace_hook.exists(),
            "{hook} should exist in marketplace Codex plugin"
        );
        assert_eq!(
            std::fs::read_to_string(root_hook).expect("read root Codex hook"),
            std::fs::read_to_string(marketplace_hook).expect("read marketplace Codex hook")
        );
    }
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

#[test]
fn installer_updates_claude_plugin_by_marketplace_selector() {
    let installer = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let installer = std::fs::read_to_string(installer).expect("read installer");

    assert!(
        installer.contains(r#"claude plugin update "magi-agent@$MAGI_PLUGIN_MARKETPLACE""#),
        "Claude updates should use the installed plugin selector including its marketplace"
    );
}

#[test]
fn installer_uses_supported_codex_plugin_commands() {
    let installer = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let installer = std::fs::read_to_string(installer).expect("read installer");

    assert!(
        !installer.contains("codex plugin update "),
        "Codex has no plugin update subcommand; re-run installation with plugin add"
    );
    assert!(
        !installer.contains("codex plugin marketplace update "),
        "Codex uses marketplace upgrade, not marketplace update"
    );
    assert!(
        installer.contains("plugin marketplace upgrade"),
        "installer should refresh Codex marketplace snapshots with marketplace upgrade"
    );
    assert!(
        installer.contains("plugin marketplace remove \"$MAGI_PLUGIN_MARKETPLACE\""),
        "installer should replace existing Codex marketplace roots before installing"
    );
    assert!(
        installer.contains("plugin remove \"magi@$MAGI_PLUGIN_MARKETPLACE\""),
        "installer should remove the existing Codex plugin before add so same-version local changes refresh"
    );
    assert!(
        installer.contains("plugin add "),
        "installer should install or refresh the Codex plugin with plugin add"
    );
}

#[test]
fn installer_skips_unusable_codex_cli() {
    let installer = Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh");
    let installer = std::fs::read_to_string(installer).expect("read installer");

    assert!(
        installer.contains("find_codex_cli()"),
        "installer should discover a usable Codex CLI instead of trusting PATH blindly"
    );
    assert!(
        installer.contains("plugin --help"),
        "installer should verify Codex plugin commands are available before installing"
    );
    assert!(
        installer.contains("skip: codex CLI found but plugin commands are unavailable"),
        "unusable Codex CLIs should be skipped without a failed plugin install warning"
    );
}
