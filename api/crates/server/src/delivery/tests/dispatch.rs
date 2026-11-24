//! The worker's look at the outbox, against fakes of both providers.

use std::time::Duration;

use rusqlite::params;

use super::support::{Behaviour, FakePush, FakeSmtp, Mood, World, minutes, open, wired_to};
use crate::delivery::dispatcher::{backoff, dispatch};
use crate::values::{Id, Timestamp};

fn soon() -> Timestamp {
    Timestamp::now().plus(minutes(1))
}

/// Ada, opted in to both, with a verified address and one browser.
async fn ada_with_everything(world: &World) -> super::support::Browser {
    world.prefer(world.ada, true, true).await;
    world.verify_email(world.ada, "ada@example.test").await;
    world.browser(world.ada, "ada").await
}

#[tokio::test]
async fn a_mention_is_sent_by_push_and_by_mail_with_generic_content() {
    let world = World::new(&[]).await;
    let browser = ada_with_everything(&world).await;
    let smtp = FakeSmtp::start(Mood::Takes).await;
    world.mail_to(smtp.port).await;
    let fake = FakePush::start(Behaviour::Status(201)).await;
    let message = world.mention(world.maja, "Hej @ada", &[world.ada]).await;
    let now = soon();

    let sent = dispatch(&world.state, &wired_to(&world, &fake), now).await.unwrap();

    assert_eq!(sent, 2);
    let mails = smtp.mails();
    assert_eq!(mails.len(), 1);
    assert!(mails[0].contains("To: ada@example.test"), "{}", mails[0]);
    assert!(mails[0].contains("Subject: New mention on Stellarr"), "{}", mails[0]);
    assert!(mails[0].contains("Message-ID: <activity-notification/"), "{}", mails[0]);
    assert!(!mails[0].contains("Hej"));
    let seen = fake.seen();
    assert_eq!(seen.len(), 1);
    let (_, plain) = open(&browser, &seen[0].body);
    let json: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(json["title"], "New mention");
    assert_eq!(json["url"], "https://chat.test/");
    assert_eq!(json["body"], "Someone mentioned you in a channel. Open Stellarr to read it.");
    let rows = world.rows(message).await;
    for row in &rows {
        assert_eq!(row.status, "sent");
        assert_eq!(row.sent_at, Some(now));
        assert_eq!(row.attempts, 1);
        assert_eq!(row.last_attempt_at, Some(now));
        assert_eq!(row.last_error, "");
        assert!(row.lease.is_none() && row.lease_until.is_none());
    }
    let push = rows.iter().find(|row| row.channel == "push").unwrap();
    assert_eq!(json["id"], push.id.to_string().replace('-', ""));
    assert_eq!(json["tag"], json["id"]);
}

#[tokio::test]
async fn a_direct_message_says_so_in_the_instances_language() {
    let world = World::new(&[("STELLARR_LANGUAGE", "pl")]).await;
    let browser = ada_with_everything(&world).await;
    let fake = FakePush::start(Behaviour::Status(201)).await;
    world.direct_message(world.maja, world.ada).await;

    dispatch(&world.state, &wired_to(&world, &fake), soon()).await.unwrap();

    let (_, plain) = open(&browser, &fake.seen()[0].body);
    let json: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(json["title"], "Nowa wiadomość prywatna");
    assert!(!String::from_utf8_lossy(&plain).contains("sekret"));
}

#[tokio::test]
async fn nothing_is_sent_for_what_changed_after_the_message() {
    for reason in [
        "membership", "message", "channel", "mute", "read", "edit", "disabled", "blocked", "preference",
        "author-is-recipient",
    ] {
        let world = World::new(&[]).await;
        ada_with_everything(&world).await;
        let fake = FakePush::start(Behaviour::Status(201)).await;
        let smtp = FakeSmtp::start(Mood::Takes).await;
        world.mail_to(smtp.port).await;
        let message = world.mention(world.maja, "@ada zobacz", &[world.ada]).await;
        let (ada, maja, server, channel) = (world.ada, world.maja, world.server, world.channel);
        world
            .write(move |tx| {
                match reason {
                    "membership" => tx.execute("DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1", [ada])?,
                    "message" => tx.execute("DELETE FROM \"Messages\" WHERE \"Id\" = ?1", [message])?,
                    "channel" => {
                        tx.execute("PRAGMA foreign_keys = OFF", [])?;
                        tx.execute("DELETE FROM \"Channels\" WHERE \"Id\" = ?1", [channel])?
                    }
                    "mute" => tx.execute(
                        "UPDATE \"UserPreferences\" SET \"MutedChannelIdsJson\" = ?2 WHERE \"UserId\" = ?1",
                        params![ada, format!("[\"{}\"]", channel.to_string().to_uppercase())],
                    )?,
                    "read" => tx.execute(
                        "INSERT INTO \"ChannelReads\" (\"ChannelId\", \"UserId\", \"LastReadAt\") VALUES (?1, ?2, ?3)",
                        params![channel, ada, Timestamp::now().plus(minutes(5))],
                    )?,
                    "edit" => tx.execute("UPDATE \"Messages\" SET \"Content\" = 'bez wzmianki' WHERE \"Id\" = ?1", [message])?,
                    "disabled" => tx.execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Id\" = ?1", [ada])?,
                    "blocked" => tx.execute(
                        "INSERT INTO \"UserBlocks\" (\"BlockerId\", \"BlockedId\", \"CreatedAt\") VALUES (?1, ?2, ?3)",
                        params![ada, maja, Timestamp::now()],
                    )?,
                    "preference" => tx.execute(
                        "UPDATE \"UserPreferences\" SET \"EmailNotifications\" = 0, \"PushNotifications\" = 0 WHERE \"UserId\" = ?1",
                        [ada],
                    )?,
                    "author-is-recipient" => tx.execute("UPDATE \"Messages\" SET \"AuthorId\" = ?2 WHERE \"Id\" = ?1", params![message, ada])?,
                    _ => unreachable!(),
                };
                let _ = server;
                Ok(())
            })
            .await;

        let sent = dispatch(&world.state, &wired_to(&world, &fake), soon()).await.unwrap();

        assert_eq!(sent, 0, "{reason}");
        assert!(fake.seen().is_empty() && smtp.mails().is_empty(), "{reason}");
        let rows = world.rows(message).await;
        assert_eq!(rows.len(), 2, "{reason}");
        assert!(rows.iter().all(|row| row.status == "cancelled"), "{reason}: {rows:?}");
    }
}

#[tokio::test]
async fn nothing_is_sent_for_a_direct_message_that_changed() {
    for reason in ["blocked", "disabled", "message", "no-direct-messages", "read", "preference"] {
        let world = World::new(&[]).await;
        ada_with_everything(&world).await;
        let smtp = FakeSmtp::start(Mood::Takes).await;
        world.mail_to(smtp.port).await;
        let fake = FakePush::start(Behaviour::Status(201)).await;
        let (conversation, message) = world.direct_message(world.maja, world.ada).await;
        let (ada, maja) = (world.ada, world.maja);
        world
            .write(move |tx| {
                match reason {
                    "blocked" => tx.execute(
                        "INSERT INTO \"UserBlocks\" (\"BlockerId\", \"BlockedId\", \"CreatedAt\") VALUES (?1, ?2, ?3)",
                        params![maja, ada, Timestamp::now()],
                    )?,
                    "disabled" => tx.execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Id\" = ?1", [ada])?,
                    "message" => tx.execute("DELETE FROM \"DirectMessages\" WHERE \"Id\" = ?1", [message])?,
                    "no-direct-messages" => tx.execute("UPDATE \"UserPreferences\" SET \"AllowDirectMessages\" = 0 WHERE \"UserId\" = ?1", [ada])?,
                    "read" => tx.execute(
                        "INSERT INTO \"DirectConversationReads\" (\"ConversationId\", \"UserId\", \"LastReadAt\") VALUES (?1, ?2, ?3)",
                        params![conversation, ada, Timestamp::now().plus(minutes(5))],
                    )?,
                    "preference" => tx.execute(
                        "UPDATE \"UserPreferences\" SET \"EmailNotifications\" = 0, \"PushNotifications\" = 0 WHERE \"UserId\" = ?1",
                        [ada],
                    )?,
                    _ => unreachable!(),
                };
                Ok(())
            })
            .await;

        dispatch(&world.state, &wired_to(&world, &fake), soon()).await.unwrap();

        assert!(fake.seen().is_empty() && smtp.mails().is_empty(), "{reason}");
        assert!(world.rows(message).await.iter().all(|row| row.status == "cancelled"), "{reason}");
    }
}

#[tokio::test]
async fn a_friend_is_still_told_when_strangers_may_not_message() {
    let world = World::new(&[]).await;
    ada_with_everything(&world).await;
    let (ada, maja) = (world.ada, world.maja);
    let fake = FakePush::start(Behaviour::Status(201)).await;
    let (_, message) = world.direct_message(maja, ada).await;
    world
        .write(move |tx| {
            tx.execute("UPDATE \"UserPreferences\" SET \"AllowDirectMessages\" = 0 WHERE \"UserId\" = ?1", [ada])?;
            let (a, b) = if ada < maja { (ada, maja) } else { (maja, ada) };
            tx.execute(
                "INSERT INTO \"Friendships\" (\"Id\", \"UserAId\", \"UserBId\", \"CreatedAt\") VALUES (?4, ?1, ?2, ?3)",
                params![a, b, Timestamp::now(), Id::random()],
            )?;
            Ok(())
        })
        .await;

    dispatch(&world.state, &wired_to(&world, &fake), soon()).await.unwrap();

    assert_eq!(fake.seen().len(), 1);
    assert!(world.rows(message).await.iter().any(|row| row.status == "sent"));
}

#[tokio::test]
async fn a_refused_push_is_tried_again_alone_with_the_backoff_and_ends_failed() {
    let world = World::new(&[]).await;
    ada_with_everything(&world).await;
    let working = FakePush::start(Behaviour::Status(201)).await;
    let message = world.mention(world.maja, "@ada", &[world.ada]).await;
    let failing = FakePush::start(Behaviour::Status(503)).await;
    let mut at = soon();
    for (attempt, wait) in [(1, 2), (2, 4), (3, 8), (4, 16), (5, 32)] {
        dispatch(&world.state, &wired_to(&world, &failing), at).await.unwrap();
        let rows = world.rows(message).await;
        let push = rows.iter().find(|row| row.channel == "push").unwrap();
        assert_eq!(push.attempts, attempt);
        assert_eq!(push.last_error, "delivery failed");
        assert_eq!(push.next_attempt_at, at.plus(minutes(wait)));
        assert_eq!(push.status, if attempt == 5 { "failed" } else { "pending" });
        assert_eq!(push.last_attempt_at, Some(at));
        at = push.next_attempt_at.plus(Duration::from_secs(1));
    }
    assert_eq!(backoff(6), Duration::from_secs(3600).min(minutes(64)));
    // The mail row never had a mail server and was only put off.
    assert!(working.seen().is_empty());
}

#[tokio::test]
async fn one_destination_failing_does_not_resend_the_other() {
    let world = World::new(&[]).await;
    world.prefer(world.ada, false, true).await;
    world.browser(world.ada, "failing").await;
    world.browser(world.ada, "working").await;
    let message = world.mention(world.maja, "@ada", &[world.ada]).await;
    let fake = FakePush::start(Behaviour::Split("/failing", 500, 201)).await;
    let sender = wired_to(&world, &fake);
    let now = soon();

    assert_eq!(dispatch(&world.state, &sender, now).await.unwrap(), 1);

    let rows = world.rows(message).await;
    let pending: Vec<_> = rows.iter().filter(|row| row.status == "pending").collect();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].attempts, 1);
    assert_eq!(rows.iter().filter(|row| row.status == "sent").count(), 1);
    // The failing one is the only one tried at its time.
    let before = fake.seen().len();
    dispatch(&world.state, &sender, pending[0].next_attempt_at.plus(Duration::from_secs(1))).await.unwrap();
    assert_eq!(fake.seen().len(), before + 1);
    assert_eq!(fake.seen().last().unwrap().path, "/failing");
}

#[tokio::test]
async fn a_gone_subscription_is_removed_and_its_row_cancelled() {
    for status in [404, 410] {
        let world = World::new(&[]).await;
        world.prefer(world.ada, false, true).await;
        let browser = world.browser(world.ada, "gone").await;
        let other = world.browser(world.ada, "alive").await;
        let message = world.mention(world.maja, "@ada", &[world.ada]).await;
        let fake = FakePush::start(Behaviour::Status(status)).await;

        let sent = dispatch(&world.state, &wired_to(&world, &fake), soon()).await.unwrap();

        assert_eq!(sent, 0);
        assert!(world.rows(message).await.iter().all(|row| row.status == "cancelled" && row.last_error.is_empty()));
        assert_eq!(world.subscriptions(world.ada).await, 0, "both answered gone");
        let _ = (browser, other);
        // A later message has no destination left.
        let later = world.mention(world.maja, "@ada again", &[world.ada]).await;
        assert!(world.rows(later).await.is_empty());
    }
}

#[tokio::test]
async fn a_row_past_its_week_is_cancelled_and_an_old_finished_one_is_deleted() {
    let world = World::new(&[]).await;
    world.prefer(world.ada, false, true).await;
    world.browser(world.ada, "ada").await;
    let fake = FakePush::start(Behaviour::Status(201)).await;
    let message = world.mention(world.maja, "@ada", &[world.ada]).await;
    let old = Id::random();
    let (ada, now) = (world.ada, Timestamp::now());
    world
        .write(move |tx| {
            tx.execute(
                "INSERT INTO \"NotificationDeliveries\" (\"Id\", \"SourceMessageId\", \"RecipientId\", \"EventKind\", \"Channel\", \"DestinationKey\",
                     \"CreatedAt\", \"ExpiresAt\", \"Attempts\", \"NextAttemptAt\", \"Status\", \"LastError\")
                 VALUES (?1, ?2, ?3, 'mention', 'email', 'ADA@EXAMPLE.TEST', ?4, ?5, 0, ?4, 'cancelled', '')",
                params![old, Id::random(), ada, now.minus(Duration::from_secs(40 * 86_400)), now.minus(Duration::from_secs(39 * 86_400))],
            )?;
            Ok(())
        })
        .await;

    dispatch(&world.state, &wired_to(&world, &fake), now.plus(Duration::from_secs(8 * 86_400))).await.unwrap();

    assert!(fake.seen().is_empty());
    assert!(world.rows(message).await.iter().all(|row| row.status == "cancelled"));
    let gone: bool = world.read(move |db| db.query_row("SELECT NOT EXISTS (SELECT 1 FROM \"NotificationDeliveries\" WHERE \"Id\" = ?1)", [old], |r| r.get(0))).await;
    assert!(gone);
}

#[tokio::test]
async fn mail_that_is_not_set_up_is_put_off_for_an_hour_without_a_try() {
    for saved in [false, true] {
        let world = World::new(&[]).await;
        world.prefer(world.ada, true, false).await;
        world.verify_email(world.ada, "ada@example.test").await;
        if saved {
            // Saved, but switched off.
            world.mail_to(1).await;
            world.write(|tx| tx.execute("UPDATE \"OperationsSettings\" SET \"MailingEnabled\" = 0", []).map(|_| ())).await;
        }
        let fake = FakePush::start(Behaviour::Status(201)).await;
        let message = world.mention(world.maja, "@ada", &[world.ada]).await;
        let now = soon();

        let sent = dispatch(&world.state, &wired_to(&world, &fake), now).await.unwrap();

        assert_eq!(sent, 0);
        let rows = world.rows(message).await;
        assert_eq!(rows[0].status, "pending");
        assert_eq!(rows[0].attempts, 0);
        assert_eq!(rows[0].next_attempt_at, now.plus(minutes(60)));
        assert!(rows[0].last_attempt_at.is_none());
    }
}

#[tokio::test]
async fn a_mail_server_that_refuses_or_is_busy_or_is_not_there_is_a_retry() {
    for mood in [Some(Mood::Refuses), Some(Mood::Busy), None] {
        let world = World::new(&[]).await;
        world.prefer(world.ada, true, false).await;
        world.verify_email(world.ada, "ada@example.test").await;
        let port = match mood {
            Some(mood) => {
                let smtp = FakeSmtp::start(mood).await;
                let port = smtp.port;
                std::mem::forget(smtp);
                port
            }
            // Nothing listens on a port that was just given back.
            None => std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port(),
        };
        world.mail_to(port).await;
        let fake = FakePush::start(Behaviour::Status(201)).await;
        let message = world.mention(world.maja, "@ada", &[world.ada]).await;
        let now = soon();

        let sent = dispatch(&world.state, &wired_to(&world, &fake), now).await.unwrap();

        assert_eq!(sent, 0);
        let rows = world.rows(message).await;
        assert_eq!(rows[0].status, "pending");
        assert_eq!(rows[0].attempts, 1);
        assert_eq!(rows[0].last_error, "delivery failed");
        assert_eq!(rows[0].next_attempt_at, now.plus(minutes(2)));
    }
}

#[tokio::test]
async fn mail_needs_the_same_verified_address_when_it_is_sent() {
    for change in ["changed", "revoked"] {
        let world = World::new(&[]).await;
        world.prefer(world.ada, true, false).await;
        world.verify_email(world.ada, "ada@example.test").await;
        let smtp = FakeSmtp::start(Mood::Takes).await;
        world.mail_to(smtp.port).await;
        let fake = FakePush::start(Behaviour::Status(201)).await;
        let message = world.mention(world.maja, "@ada", &[world.ada]).await;
        let ada = world.ada;
        world
            .write(move |tx| {
                if change == "changed" {
                    tx.execute("UPDATE \"Users\" SET \"Email\" = 'new@example.test', \"NormalizedEmail\" = 'NEW@EXAMPLE.TEST' WHERE \"Id\" = ?1", [ada])?;
                } else {
                    tx.execute("UPDATE \"MfaMethods\" SET \"RevokedAt\" = ?2 WHERE \"UserId\" = ?1", params![ada, Timestamp::now()])?;
                }
                Ok(())
            })
            .await;

        dispatch(&world.state, &wired_to(&world, &fake), soon()).await.unwrap();

        assert!(smtp.mails().is_empty(), "{change}");
        assert_eq!(world.rows(message).await[0].status, "cancelled");
    }
}

#[tokio::test]
async fn a_push_needs_a_live_subscription_of_the_recipient() {
    for state in ["deleted", "revoked", "expired", "version", "owner"] {
        let world = World::new(&[]).await;
        world.prefer(world.ada, false, true).await;
        let browser = world.browser(world.ada, "ada").await;
        let fake = FakePush::start(Behaviour::Status(201)).await;
        let message = world.mention(world.maja, "@ada", &[world.ada]).await;
        let (id, session, ada, maja) = (browser.id, browser.session, world.ada, world.maja);
        world
            .write(move |tx| {
                match state {
                    "deleted" => tx.execute("DELETE FROM \"PushSubscriptions\" WHERE \"Id\" = ?1", [id])?,
                    "revoked" => tx.execute("UPDATE \"UserSessions\" SET \"RevokedAt\" = ?2 WHERE \"Id\" = ?1", params![session, Timestamp::now()])?,
                    "expired" => tx.execute("UPDATE \"UserSessions\" SET \"ExpiresAt\" = ?2, \"AbsoluteExpiresAt\" = ?2 WHERE \"Id\" = ?1", params![session, Timestamp::now().minus(minutes(1))])?,
                    "version" => tx.execute("UPDATE \"Users\" SET \"SessionVersion\" = 1 WHERE \"Id\" = ?1", [ada])?,
                    "owner" => tx.execute("UPDATE \"PushSubscriptions\" SET \"UserId\" = ?2 WHERE \"Id\" = ?1", params![id, maja])?,
                    _ => unreachable!(),
                };
                Ok(())
            })
            .await;

        dispatch(&world.state, &wired_to(&world, &fake), soon()).await.unwrap();

        assert!(fake.seen().is_empty(), "{state}");
        assert_eq!(world.rows(message).await[0].status, "cancelled", "{state}");
    }
}

#[tokio::test]
async fn two_looks_at_once_send_each_row_once() {
    let world = World::new(&[]).await;
    world.prefer(world.ada, false, true).await;
    let mut browsers = Vec::new();
    for index in 0..6 {
        browsers.push(world.browser(world.ada, &format!("b{index}")).await);
    }
    let message = world.mention(world.maja, "@ada", &[world.ada]).await;
    let fake = FakePush::start(Behaviour::Status(201)).await;
    let (first, second) = (wired_to(&world, &fake), wired_to(&world, &fake));
    let now = soon();

    let (a, b) = tokio::join!(
        dispatch(&world.state, &first, now),
        dispatch(&world.state, &second, now)
    );

    assert_eq!(a.unwrap() + b.unwrap(), 6);
    assert_eq!(fake.seen().len(), 6);
    let rows = world.rows(message).await;
    assert!(rows.iter().all(|row| row.status == "sent" && row.attempts == 1));
}

#[tokio::test]
async fn a_batch_is_twenty_five_rows_in_the_order_they_became_due() {
    let world = World::new(&[]).await;
    world.prefer(world.ada, false, true).await;
    for index in 0..30 {
        world.browser(world.ada, &format!("b{index}")).await;
    }
    let message = world.mention(world.maja, "@ada", &[world.ada]).await;
    let fake = FakePush::start(Behaviour::Status(201)).await;
    let sender = wired_to(&world, &fake);
    let now = soon();

    assert_eq!(dispatch(&world.state, &sender, now).await.unwrap(), 25);
    assert_eq!(dispatch(&world.state, &sender, now).await.unwrap(), 5);
    assert_eq!(world.rows(message).await.len(), 30);
}

#[tokio::test]
async fn a_row_that_is_leased_waits_for_its_lease_and_one_tried_five_times_is_left_alone() {
    let world = World::new(&[]).await;
    world.prefer(world.ada, false, true).await;
    world.browser(world.ada, "a").await;
    let fake = FakePush::start(Behaviour::Status(201)).await;
    let sender = wired_to(&world, &fake);
    let message = world.mention(world.maja, "@ada", &[world.ada]).await;
    let row = world.rows(message).await[0].id;
    let now = soon();
    // A look that sent the push and stopped before writing: leased, one try.
    world
        .write(move |tx| {
            tx.execute(
                "UPDATE \"NotificationDeliveries\" SET \"LeaseId\" = ?2, \"LeaseUntil\" = ?3, \"Attempts\" = 1 WHERE \"Id\" = ?1",
                params![row, Id::random(), now.plus(minutes(2))],
            )?;
            Ok(())
        })
        .await;

    assert_eq!(dispatch(&world.state, &sender, now.plus(minutes(1))).await.unwrap(), 0);
    assert!(fake.seen().is_empty());
    // Once the lease has run out it is sent, and counted again.
    assert_eq!(dispatch(&world.state, &sender, now.plus(minutes(3))).await.unwrap(), 1);
    let after = &world.rows(message).await[0];
    assert_eq!((after.attempts, after.status.as_str()), (2, "sent"));

    let tired = world.mention(world.maja, "@ada", &[world.ada]).await;
    let tired_row = world.rows(tired).await[0].id;
    world
        .write(move |tx| tx.execute("UPDATE \"NotificationDeliveries\" SET \"Attempts\" = 5 WHERE \"Id\" = ?1", [tired_row]).map(|_| ()))
        .await;
    assert_eq!(dispatch(&world.state, &sender, now.plus(minutes(10))).await.unwrap(), 0);
    assert_eq!(world.rows(tired).await[0].status, "pending");
}

#[tokio::test]
async fn a_clock_that_goes_back_does_not_send_what_is_not_yet_due() {
    let world = World::new(&[]).await;
    world.prefer(world.ada, false, true).await;
    world.browser(world.ada, "a").await;
    let message = world.mention(world.maja, "@ada", &[world.ada]).await;
    let failing = FakePush::start(Behaviour::Status(503)).await;
    let now = soon();
    dispatch(&world.state, &wired_to(&world, &failing), now).await.unwrap();
    let working = FakePush::start(Behaviour::Status(201)).await;

    // An hour earlier than the failure: the row waits for its own time.
    let sent = dispatch(&world.state, &wired_to(&world, &working), now.minus(minutes(60))).await.unwrap();

    assert_eq!(sent, 0);
    assert!(working.seen().is_empty());
    assert_eq!(world.rows(message).await[0].attempts, 1);
}

/// A writer that keeps every line the log gets.
#[derive(Clone, Default)]
struct Lines(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Lines {
    type Writer = Lines;

    fn make_writer(&'a self) -> Lines {
        self.clone()
    }
}

#[tokio::test]
async fn a_mail_that_cannot_be_written_logs_the_row_and_never_the_address() {
    use tracing::instrument::WithSubscriber;

    let world = World::new(&[]).await;
    world.prefer(world.ada, true, false).await;
    // An address the mail library refuses, and names in its error.
    world.verify_email(world.ada, "ada-without-an-at-sign.example").await;
    let smtp = FakeSmtp::start(Mood::Takes).await;
    world.mail_to(smtp.port).await;
    let fake = FakePush::start(Behaviour::Status(201)).await;
    let message = world.mention(world.maja, "@ada", &[world.ada]).await;
    let lines = Lines::default();
    let sender = wired_to(&world, &fake);
    let mut at = soon();

    // Every try the row gets (it gets five) is logged.
    for _ in 0..5 {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(lines.clone())
            .with_ansi(false)
            .finish();
        dispatch(&world.state, &sender, at)
            .with_subscriber(subscriber)
            .await
            .unwrap();
        at = at.plus(minutes(70));
    }

    let log = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
    let row = world.rows(message).await.remove(0);
    assert_eq!((row.status.as_str(), row.attempts), ("failed", 5), "the mail did fail every time");
    assert_eq!(
        log.matches(&format!("An activity notification delivery {} failed: the mail could not be written.", row.id)).count(),
        5,
        "{log}"
    );
    assert!(!log.contains("without-an-at-sign") && !log.contains("example"), "{log}");
}

#[tokio::test]
async fn every_row_of_a_slow_batch_keeps_its_lease_for_as_long_as_the_batch_can_last() {
    let world = World::new(&[]).await;
    world.prefer(world.ada, false, true).await;
    for index in 0..25 {
        world.browser(world.ada, &format!("b{index}")).await;
    }
    let message = world.mention(world.maja, "@ada", &[world.ada]).await;
    let fake = FakePush::start(Behaviour::Slow(400)).await;
    let sender = wired_to(&world, &fake);
    let now = soon();

    let looking = dispatch(&world.state, &sender, now);
    let watching = async {
        // While the batch is under way: the shortest lease any row still holds.
        let mut shortest = None;
        for _ in 0..8 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let leases: Vec<Timestamp> = world
                .read(|db| {
                    db.prepare("SELECT \"LeaseUntil\" FROM \"NotificationDeliveries\" WHERE \"LeaseId\" IS NOT NULL")?
                        .query_map([], |row| row.get(0))?
                        .collect()
                })
                .await;
            for lease in leases {
                shortest = Some(shortest.map_or(lease, |known: Timestamp| known.min(lease)));
            }
        }
        shortest
    };
    let (sent, shortest) = tokio::join!(looking, watching);

    assert_eq!(sent.unwrap(), 25);
    let shortest = shortest.expect("the batch was under way when it was looked at");
    // The 25th row is reached after 24 others at their worst, then has its own
    // ten seconds: the lease must reach past the whole of it.
    assert!(
        shortest >= now.plus(Duration::from_secs(25 * 20)),
        "a row of the batch holds its lease only until {shortest}, from {now}"
    );
    assert!(world.rows(message).await.iter().all(|row| row.status == "sent"));
}
