//! Tests for the ephemeral session-scoped agent naming and lifecycle.
//!
//! The naming-core tests are pure (no Redis): they pin the deterministic MAGI
//! suffix cycle and the adjective re-roll / fallback behavior by injecting the
//! adjective source and the name-claim predicate. The lifecycle tests exercise
//! `spawn` / `despawn` against an ephemeral Redis container via `redis_fixture`.

use magi::agent::{
    candidate_names, compose_name, despawn_with_url, magi_suffix, random_adjective, spawn_with_url,
    MAGI_SUFFIXES,
};
use magi::error::MagiError;
use magi::model::RedisKeys;
use magi::team::{create_team_with_url, list_members_with_url};

use rand::rngs::StdRng;
use rand::SeedableRng;
use redis::AsyncCommands;

mod common;
use common::{redis_fixture, RedisFixture};
use rstest::rstest;

/// Returns a name unique within this test run, isolating teams across the cases
/// that share one Redis instance.
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

/// Opens a multiplexed async Redis connection to `url` for test setup.
async fn redis_connection(url: &str) -> redis::aio::MultiplexedConnection {
    redis::Client::open(url)
        .expect("redis client")
        .get_multiplexed_async_connection()
        .await
        .expect("redis connection")
}

// --- Adjective source (petname word list, our RNG) ---

/// The adjective is drawn deterministically for a given seed and is a single
/// clean word (non-empty, no whitespace) suitable as a name segment.
#[test]
fn random_adjective_is_deterministic_and_clean() {
    use rand::SeedableRng;
    let a = magi::agent::random_adjective(&mut rand::rngs::StdRng::seed_from_u64(7));
    let b = magi::agent::random_adjective(&mut rand::rngs::StdRng::seed_from_u64(7));
    assert_eq!(a, b, "same seed must yield the same adjective");
    assert!(!a.is_empty(), "adjective must not be empty");
    assert!(
        !a.chars().any(char::is_whitespace),
        "adjective must be a single word: {a:?}"
    );
}

// --- Pure naming-core tests (no Redis required) ---

/// The suffix must cycle melchior -> balthasar -> casper and wrap, driven by a
/// 1-based monotonic sequence number.
#[test]
fn magi_suffix_cycles_in_fixed_order() {
    assert_eq!(MAGI_SUFFIXES, ["melchior", "balthasar", "casper"]);
    assert_eq!(magi_suffix(1), "melchior");
    assert_eq!(magi_suffix(2), "balthasar");
    assert_eq!(magi_suffix(3), "casper");
    assert_eq!(magi_suffix(4), "melchior");
    assert_eq!(magi_suffix(5), "balthasar");
    assert_eq!(magi_suffix(6), "casper");
}

/// A `seq` of 0 (never produced by `INCR`, but guarded) maps to the first unit.
#[test]
fn magi_suffix_treats_zero_as_first_unit() {
    assert_eq!(magi_suffix(0), "melchior");
}

/// Names join the adjective and suffix with a single hyphen.
#[test]
fn compose_name_joins_with_hyphen() {
    assert_eq!(compose_name("quiet", "melchior"), "quiet-melchior");
}

/// The candidate list is `max_attempts` `<adjective>-<suffix>` names followed by
/// a single `<adjective>-<suffix>-<seq>` fallback, drawing adjectives in order.
#[test]
fn candidate_names_lists_base_attempts_then_seq_fallback() {
    let mut adjectives = ["alpha", "bravo", "charlie"].into_iter().map(String::from);
    let names = candidate_names(2, 2, || adjectives.next().unwrap());
    assert_eq!(
        names,
        vec![
            "alpha-balthasar".to_string(),     // attempt 1
            "bravo-balthasar".to_string(),     // attempt 2
            "charlie-balthasar-2".to_string(), // seq fallback
        ]
    );
}

/// With a single attempt, the list is one base candidate plus the fallback, and
/// the suffix follows the deterministic cycle for the given sequence number.
#[test]
fn candidate_names_uses_cycle_suffix_and_appends_fallback() {
    let mut adjectives = ["solo", "back"].into_iter().map(String::from);
    let names = candidate_names(3, 1, || adjectives.next().unwrap());
    assert_eq!(
        names,
        vec!["solo-casper".to_string(), "back-casper-3".to_string()]
    );
}

// --- Redis-backed lifecycle tests (ephemeral container via redis_fixture) ---

/// Three successive spawns into the same team receive the MAGI suffixes in
/// cycle order and are all registered as team members.
#[rstest]
#[tokio::test]
async fn spawn_assigns_cyclic_magi_suffixes(#[future(awt)] redis_fixture: RedisFixture) {
    let url = redis_fixture.url();
    let team = unique_name("spawn-cycle");
    create_team_with_url(url, &team, "owner")
        .await
        .expect("create team");

    let mut rng = StdRng::seed_from_u64(1);
    let first = spawn_with_url(url, &team, "claude-code", "/proj", &mut rng)
        .await
        .expect("spawn 1");
    let second = spawn_with_url(url, &team, "claude-code", "/proj", &mut rng)
        .await
        .expect("spawn 2");
    let third = spawn_with_url(url, &team, "claude-code", "/proj", &mut rng)
        .await
        .expect("spawn 3");

    assert!(first.ends_with("-melchior"), "first was {first}");
    assert!(second.ends_with("-balthasar"), "second was {second}");
    assert!(third.ends_with("-casper"), "third was {third}");

    let members = list_members_with_url(url, &team)
        .await
        .expect("list members");
    let names: Vec<&str> = members.iter().map(|m| m.name.as_str()).collect();
    for spawned in [&first, &second, &third] {
        assert!(
            names.contains(&spawned.as_str()),
            "{spawned} should be a member"
        );
    }
}

/// When the first candidate name is already taken, spawn re-rolls to a free
/// adjective while keeping the deterministic suffix for that sequence number.
#[rstest]
#[tokio::test]
async fn spawn_rerolls_past_a_taken_name(#[future(awt)] redis_fixture: RedisFixture) {
    let url = redis_fixture.url();
    let team = unique_name("spawn-reroll");
    create_team_with_url(url, &team, "owner")
        .await
        .expect("create team");

    // The first spawn uses seq=1 (suffix melchior); predict its first candidate
    // under this seed and occupy that exact name so spawn must re-roll.
    let occupied = format!(
        "{}-melchior",
        random_adjective(&mut StdRng::seed_from_u64(9))
    );
    let keys = RedisKeys::new(&team);
    let mut conn = redis_connection(url).await;
    let _: i64 = conn
        .sadd(keys.team_agents(), &occupied)
        .await
        .expect("occupy name");

    let mut rng = StdRng::seed_from_u64(9);
    let name = spawn_with_url(url, &team, "claude-code", "/proj", &mut rng)
        .await
        .expect("spawn");

    assert_ne!(name, occupied, "must not reuse the occupied name");
    assert!(name.ends_with("-melchior"), "still seq=1 suffix: {name}");
}

/// Despawn removes the agent from the roster; a second despawn of the same
/// agent reports it is no longer a member.
#[rstest]
#[tokio::test]
async fn despawn_removes_member_and_is_not_found_twice(#[future(awt)] redis_fixture: RedisFixture) {
    let url = redis_fixture.url();
    let team = unique_name("despawn");
    create_team_with_url(url, &team, "owner")
        .await
        .expect("create team");

    let mut rng = StdRng::seed_from_u64(3);
    let name = spawn_with_url(url, &team, "claude-code", "/proj", &mut rng)
        .await
        .expect("spawn");

    despawn_with_url(url, &team, &name).await.expect("despawn");

    let members = list_members_with_url(url, &team)
        .await
        .expect("list members");
    assert!(
        !members.iter().any(|m| m.name == name),
        "{name} should have been removed"
    );

    // Removing again is a not-found, which the CLI layer treats as idempotent.
    let err = despawn_with_url(url, &team, &name)
        .await
        .expect_err("second despawn should be not-found");
    assert!(matches!(err, MagiError::NotFound(_)), "got {err:?}");
}
