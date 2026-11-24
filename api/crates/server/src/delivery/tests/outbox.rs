use std::time::Duration;

use super::support::{World, at};
use crate::delivery::outbox::{self, Kind};
use crate::values::Id;

#[tokio::test]
async fn a_mention_queues_one_row_for_each_destination_that_is_opted_in() {
    let world = World::new(&[]).await;
    let ilya = world.member("ilya").await;
    let nils = world.member("nils").await;
    world.prefer(world.ada, true, true).await;
    world.verify_email(world.ada, "ada@example.test").await;
    let browser = world.browser(world.ada, "ada").await;
    // Ilya opted in but has no address and no browser; Nils never opted in.
    world.prefer(ilya, true, true).await;
    world.prefer(nils, false, false).await;

    let message = world
        .mention(world.maja, "Hej @ada @ilya @nils", &[world.ada, ilya, nils, world.ada])
        .await;

    let rows = world.rows(message).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.iter().all(|row| row.recipient == world.ada && row.kind == "mention"));
    let email = &rows[0];
    assert_eq!(email.channel, "email");
    assert_eq!(email.key, "ADA@EXAMPLE.TEST");
    assert_eq!(email.subscription, None);
    let push = &rows[1];
    assert_eq!(push.channel, "push");
    assert_eq!(push.subscription, Some(browser.id));
    assert_eq!(push.key, browser.id.to_string().replace('-', ""));
    for row in &rows {
        assert_eq!(row.status, "pending");
        assert_eq!(row.attempts, 0);
        assert_eq!(row.next_attempt_at, row.created_at);
        assert_eq!(row.expires_at, row.created_at.plus(Duration::from_secs(7 * 86_400)));
        assert_eq!(row.last_error, "");
        assert!(row.sent_at.is_none() && row.lease.is_none() && row.last_attempt_at.is_none());
    }
}

#[tokio::test]
async fn a_direct_message_is_owed_to_the_peer_as_its_own_kind() {
    let world = World::new(&[]).await;
    world.prefer(world.ada, false, true).await;
    world.browser(world.ada, "ada").await;

    let (_, message) = world.direct_message(world.maja, world.ada).await;

    let rows = world.rows(message).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "direct-message");
    assert_eq!(Kind::parse(&rows[0].kind), Some(Kind::DirectMessage));
}

#[tokio::test]
async fn a_browser_whose_sign_in_has_ended_gets_no_row() {
    let world = World::new(&[]).await;
    world.prefer(world.ada, false, true).await;
    let ended = world.browser(world.ada, "ended").await;
    let moved = world.browser(world.ada, "version").await;
    let session = ended.session;
    let other = moved.session;
    world
        .write(move |tx| {
            tx.execute("UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1", rusqlite::params![session, at("2026-01-01 00:00:00")])?;
            tx.execute("UPDATE \"UserSessions\" SET \"SecurityVersion\" = 9 WHERE \"Id\" = ?1", [other])?;
            Ok(())
        })
        .await;

    let message = world.mention(world.maja, "@ada", &[world.ada]).await;

    assert!(world.rows(message).await.is_empty());
}

#[tokio::test]
async fn a_disabled_account_and_an_unknown_one_get_nothing() {
    let world = World::new(&[]).await;
    world.prefer(world.ada, false, true).await;
    world.browser(world.ada, "ada").await;
    let ada = world.ada;
    world
        .write(move |tx| tx.execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Id\" = ?1", [ada]).map(|_| ()))
        .await;

    let message = world.mention(world.maja, "@ada", &[world.ada, Id::random()]).await;

    assert!(world.rows(message).await.is_empty());
}

#[test]
fn destination_keys_are_trimmed_and_uppercase() {
    assert_eq!(outbox::destination_key("  Ada@Example.test "), "ADA@EXAMPLE.TEST");
    // The dotless i is not capitalized, and the long s is.
    assert_eq!(outbox::destination_key("ı@ſ.test"), "ı@S.TEST");
    assert_eq!(
        outbox::push_key(Id::parse("8F62A4D1-BDCB-4D86-BD1C-6E351900E629").unwrap()),
        "8f62a4d1bdcb4d86bd1c6e351900e629"
    );
}
