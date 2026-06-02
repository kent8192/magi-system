//! Redis-backed coverage for upstream agmsg CLI parity features.

use magi::actas::{claim_with_url, release_with_url, status_with_url};
use magi::messaging::{history_with_url, send_message_with_url};
use magi::team::{
    create_team_with_url, list_members_with_url, list_registrations_with_url,
    register_agent_scoped_with_url, register_agent_with_url, rename_agent_with_url,
    rename_team_with_url, reset_registrations_with_url, suggested_registrations_with_url,
};

mod common;
use common::{redis_fixture, RedisFixture};
use rstest::rstest;

fn unique_name(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    )
}

#[rstest]
#[tokio::test]
async fn registration_reset_removes_matching_tuple_without_deleting_other_registration(
    #[future(awt)] redis_fixture: RedisFixture,
) {
    let url = redis_fixture.url().to_string();
    let team = unique_name("team-registration-reset");
    let agent = unique_name("agent");
    let project_a = format!("/tmp/{}", unique_name("project-a"));
    let project_b = format!("/tmp/{}", unique_name("project-b"));
    create_team_with_url(&url, &team, &agent).await.unwrap();
    register_agent_scoped_with_url(&url, &team, &agent, "codex", &project_a, Some("s1"))
        .await
        .unwrap();
    register_agent_scoped_with_url(&url, &team, &agent, "codex", &project_b, Some("s2"))
        .await
        .unwrap();

    let removed = reset_registrations_with_url(&url, &project_a, "codex", Some(&agent), Some("s1"))
        .await
        .unwrap();

    assert_eq!(removed, 1);
    let exact = list_registrations_with_url(&url, &project_a, "codex")
        .await
        .unwrap();
    assert!(exact.is_empty());
    let remaining = list_registrations_with_url(&url, &project_b, "codex")
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].agent, agent);
}

#[rstest]
#[tokio::test]
async fn identity_discovery_returns_exact_and_suggested_project_type_matches(
    #[future(awt)] redis_fixture: RedisFixture,
) {
    let url = redis_fixture.url().to_string();
    let team = unique_name("team-identity");
    let exact_agent = unique_name("exact");
    let suggested_agent = unique_name("suggested");
    let project_a = format!("/tmp/{}", unique_name("project-a"));
    let project_b = format!("/tmp/{}", unique_name("project-b"));
    create_team_with_url(&url, &team, &exact_agent)
        .await
        .unwrap();
    register_agent_with_url(&url, &team, &exact_agent, "codex", &project_a)
        .await
        .unwrap();
    register_agent_with_url(&url, &team, &suggested_agent, "codex", &project_b)
        .await
        .unwrap();

    let exact = list_registrations_with_url(&url, &project_a, "codex")
        .await
        .unwrap();
    assert!(exact
        .iter()
        .any(|registration| registration.agent == exact_agent));
    let suggested = suggested_registrations_with_url(&url, &project_a, "codex")
        .await
        .unwrap();
    assert!(suggested
        .iter()
        .any(|registration| registration.agent == suggested_agent));
}

#[rstest]
#[tokio::test]
async fn agent_rename_moves_profile_registrations_and_cursor_but_keeps_history_names(
    #[future(awt)] redis_fixture: RedisFixture,
) {
    let url = redis_fixture.url().to_string();
    let team = unique_name("team-agent-rename");
    let alice = unique_name("alice");
    let bob = unique_name("bob");
    let renamed = unique_name("renamed");
    create_team_with_url(&url, &team, &alice).await.unwrap();
    register_agent_with_url(&url, &team, &bob, "codex", "/tmp/project")
        .await
        .unwrap();
    send_message_with_url(&url, &team, &alice, &bob, "hello")
        .await
        .unwrap();

    rename_agent_with_url(&url, &team, &bob, &renamed)
        .await
        .unwrap();

    let members = list_members_with_url(&url, &team).await.unwrap();
    assert!(members.iter().any(|member| member.name == renamed));
    assert!(!members.iter().any(|member| member.name == bob));
    let history = history_with_url(&url, &team, Some(&bob), None)
        .await
        .unwrap();
    assert_eq!(history.len(), 1, "history remains immutable");
}

#[rstest]
#[tokio::test]
async fn team_rename_moves_roster_and_stream_history(#[future(awt)] redis_fixture: RedisFixture) {
    let url = redis_fixture.url().to_string();
    let old_team = unique_name("team-old");
    let new_team = unique_name("team-new");
    let alice = unique_name("alice");
    let bob = unique_name("bob");
    create_team_with_url(&url, &old_team, &alice).await.unwrap();
    register_agent_with_url(&url, &old_team, &bob, "codex", "/tmp/project")
        .await
        .unwrap();
    send_message_with_url(&url, &old_team, &alice, &bob, "hello")
        .await
        .unwrap();

    rename_team_with_url(&url, &old_team, &new_team)
        .await
        .unwrap();

    let members = list_members_with_url(&url, &new_team).await.unwrap();
    assert!(members.iter().any(|member| member.name == alice));
    let history = history_with_url(&url, &new_team, None, None).await.unwrap();
    assert_eq!(history.len(), 1);
}

#[rstest]
#[tokio::test]
async fn actas_claim_rejects_other_session_and_releases_owner(
    #[future(awt)] redis_fixture: RedisFixture,
) {
    let url = redis_fixture.url().to_string();
    let team = unique_name("team-actas");
    let agent = unique_name("agent");
    create_team_with_url(&url, &team, &agent).await.unwrap();

    claim_with_url(&url, &team, &agent, "session-a", 60)
        .await
        .unwrap();
    let error = claim_with_url(&url, &team, &agent, "session-b", 60)
        .await
        .expect_err("other session should not steal claim");
    assert!(error.to_string().contains("already claimed"));

    assert_eq!(
        status_with_url(&url, &team, &agent)
            .await
            .unwrap()
            .as_deref(),
        Some("session-a")
    );
    release_with_url(&url, &team, &agent, "session-a")
        .await
        .unwrap();
    assert!(status_with_url(&url, &team, &agent)
        .await
        .unwrap()
        .is_none());
}
