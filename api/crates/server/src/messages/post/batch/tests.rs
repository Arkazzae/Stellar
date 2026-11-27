use super::*;
use crate::permissions::fixture::World;
use stellarr_messages::Language;

// Fill the queue before starting the worker so all three posts share the same transaction.
async fn post_three(world: &World) -> ([bool; 3], Vec<String>) {
    let (sender, receiver) = mpsc::channel(BATCH_LIMIT);
    let mut replies = Vec::new();
    for content in ["first", "second", "third"] {
        let (reply, answer) = oneshot::channel();
        sender
            .try_send(Job {
                server: world.server,
                submission: Submission {
                    channel: world.general,
                    author: world.ilya,
                    content: content.to_owned(),
                    attachments: Vec::new(),
                    reply_to: None,
                    client_id: Some(Id::random()),
                    scheduled: None,
                    poll: None,
                    event: None,
                },
                reply,
            })
            .unwrap();
        replies.push(answer);
    }
    drop(sender);
    run(
        world.database.clone(),
        Messages::new(Language::English),
        receiver,
        Arc::default(),
    )
    .await;

    let mut saved = [false; 3];
    for (result, reply) in saved.iter_mut().zip(replies) {
        *result = match reply.await.unwrap() {
            Ok(Posted::Stored(_)) => true,
            Ok(Posted::Replayed(_)) => panic!("each test post has a different client ID"),
            Err(_) => false,
        };
    }
    let stored = world
        .db
        .prepare("SELECT Content FROM Messages WHERE ChannelId = ?1 ORDER BY rowid")
        .unwrap()
        .query_map([world.general], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    (saved, stored)
}

#[tokio::test]
async fn a_failed_post_rolls_back_its_own_insert() {
    let world = World::new();
    world
        .db
        .execute_batch(
            "CREATE TRIGGER FailSecond AFTER INSERT ON Messages WHEN NEW.Content = 'second'
         BEGIN SELECT RAISE(FAIL, 'failed after insert'); END;",
        )
        .unwrap();
    let (saved, stored) = post_three(&world).await;
    assert_eq!(saved, [true, false, true]);
    assert_eq!(stored, ["first", "third"]);
}

#[tokio::test]
async fn a_failed_commit_rejects_every_post_in_the_transaction() {
    let world = World::new();
    world
        .db
        .execute_batch(
            "CREATE TABLE Parent (Id INTEGER PRIMARY KEY);
         CREATE TABLE Child (Id INTEGER REFERENCES Parent(Id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TRIGGER FailCommit AFTER INSERT ON Messages WHEN NEW.Content = 'second'
         BEGIN INSERT INTO Child VALUES (1); END;",
        )
        .unwrap();
    let (saved, stored) = post_three(&world).await;
    assert_eq!(saved, [false; 3]);
    assert!(stored.is_empty());
}
