use std::path::Path;

use serde_json::Value;

#[test]
fn devcontainer_exposes_host_magi_codex_runtime_surfaces() {
    let devcontainer_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".devcontainer/devcontainer.json");
    let devcontainer: Value = serde_json::from_str(
        &std::fs::read_to_string(devcontainer_path).expect("read devcontainer config"),
    )
    .expect("devcontainer config should be valid JSON");

    let mounts = devcontainer["mounts"]
        .as_array()
        .expect("devcontainer mounts should be an array");
    assert_mount(
        mounts,
        "source=${localEnv:HOME}/.magi,target=/home/vscode/.magi,type=bind",
    );
    assert_mount(
        mounts,
        "source=${localEnv:HOME}/.local/state/magi-codex,target=/home/vscode/.local/state/magi-codex,type=bind",
    );
    assert_mount(
        mounts,
        "source=${localEnv:HOME}/.codex/app-server-control,target=/home/vscode/.codex/app-server-control,type=bind",
    );

    let remote_env = devcontainer["remoteEnv"]
        .as_object()
        .expect("devcontainer remoteEnv should be an object");
    assert_eq!(
        remote_env["MAGI_CODEX_STATE_DIR"],
        "/home/vscode/.local/state/magi-codex"
    );
    assert!(!remote_env.contains_key("MAGI_CODEX_APP_SERVER_SOCKET"));
}

fn assert_mount(mounts: &[Value], expected_prefix: &str) {
    assert!(
        mounts
            .iter()
            .filter_map(Value::as_str)
            .any(|mount| mount.starts_with(expected_prefix)),
        "expected devcontainer mount starting with `{expected_prefix}`"
    );
}
