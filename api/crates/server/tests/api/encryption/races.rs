//! What happens when two requests meet: each of these is one transaction that
//! waits its turn, so each pair is settled one after the other and never
//! both win.

use std::future::Future;
use std::sync::Arc;

use rusqlite::params;
use serde_json::{Value, json};
use stellarr_api::values::Id;

use super::kit::*;
use super::limits::seed_devices;
use crate::support::{Client, Response};

/// Sends every request at once and says what each was answered.
async fn together<F>(requests: Vec<F>) -> Vec<Response>
where
    F: Future<Output = Response> + Send + 'static,
{
    let mut tasks = tokio::task::JoinSet::new();
    for request in requests {
        tasks.spawn(request);
    }
    let mut answers = Vec::new();
    while let Some(answer) = tasks.join_next().await {
        answers.push(answer.unwrap());
    }
    answers
}

fn statuses(answers: &[Response]) -> Vec<u16> {
    let mut statuses: Vec<u16> = answers.iter().map(|a| a.status.as_u16()).collect();
    statuses.sort_unstable();
    statuses
}

fn code_of(answers: &[Response], status: u16) -> Option<String> {
    refusal(answers.iter().find(|a| a.status.as_u16() == status).unwrap()).1
}

#[tokio::test]
async fn two_devices_registered_at_once_when_one_place_is_left_do_not_both_get_it() {
    let crowd = Crowd::new().await;
    set_up(&crowd.maja.client).await;
    seed_devices(&crowd.server, &crowd.maja.id, 99);
    let laptop = Arc::new(crowd.server.sign_in(&crowd.maja).await);
    let phone = Arc::new(crowd.server.sign_in(&crowd.maja).await);

    let answers = together(
        vec![(laptop, bytes(32, 90), "Laptop"), (phone, bytes(32, 91), "Phone")]
            .into_iter()
            .map(|(client, key, label)| tokio::spawn(async move { register_device_with(&client, &key, label).await }))
            .map(|task| async move { task.await.unwrap() })
            .collect::<Vec<_>>(),
    )
    .await;

    assert_eq!(statuses(&answers), [201, 409]);
    assert_eq!(code_of(&answers, 409).as_deref(), Some("device-limit"));
    let count: i64 = crowd
        .server
        .database()
        .query_row(
            "SELECT COUNT(*) FROM \"EncryptionDevices\" WHERE \"UserId\" = ?1 AND \"RevokedAt\" IS NULL",
            [Id::parse(&crowd.maja.id).unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 100);
}

#[tokio::test]
async fn confirming_and_cancelling_at_the_same_moment_leave_one_of_them_standing() {
    for _ in 0..3 {
        let crowd = Crowd::new().await;
        set_up(&crowd.maja.client).await;
        set_up(&crowd.ada.client).await;
        let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
        assert_eq!(action(&crowd.maja.client, &conversation, "request").await.status, 200);
        let maja = Arc::new(crowd.server.sign_in(&crowd.maja).await);
        let ada = Arc::new(crowd.server.sign_in(&crowd.ada).await);
        let (c1, c2) = (conversation.clone(), conversation.clone());

        let answers = together(vec![
            tokio::spawn(async move { action(&maja, &c1, "cancel").await }),
            tokio::spawn(async move { action(&ada, &c2, "confirm").await }),
        ]
        .into_iter()
        .map(|task| async move { task.await.unwrap() })
        .collect::<Vec<_>>())
        .await;

        assert_eq!(statuses(&answers), [200, 409]);
        assert_eq!(code_of(&answers, 409).as_deref(), Some("encryption-state"));
        let timeline = crowd.timeline(&conversation);
        let state: String = crowd
            .server
            .database()
            .query_row(
                "SELECT \"EncryptionState\" FROM \"DirectConversations\" WHERE \"Id\" = ?1",
                [Id::parse(&conversation).unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        match state.as_str() {
            "On" => assert_eq!(timeline, ["encryption_requested:maja", "encryption_enabled:ada"]),
            "Off" => assert_eq!(timeline, ["encryption_requested:maja", "encryption_request_cancelled:maja"]),
            other => panic!("{other}: {timeline:?}"),
        }
    }
}

#[tokio::test]
async fn two_epochs_that_follow_the_same_one_do_not_both_take_the_next_number() {
    let crowd = Crowd::ready().await;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;
    let every = crowd.recipients(&crowd.maja.client, &conversation).await;
    let maja = Arc::new(crowd.server.sign_in(&crowd.maja).await);
    let ada = Arc::new(crowd.server.sign_in(&crowd.ada).await);
    let a = epoch_body(&Id::random().to_string(), None, &every);
    let b = epoch_body(&Id::random().to_string(), None, &every);
    let (c1, c2) = (conversation.clone(), conversation.clone());

    let answers = together(vec![
        tokio::spawn(async move { post_epoch(&maja, &c1, &a).await }),
        tokio::spawn(async move { post_epoch(&ada, &c2, &b).await }),
    ]
    .into_iter()
    .map(|task| async move { task.await.unwrap() })
    .collect::<Vec<_>>())
    .await;

    assert_eq!(statuses(&answers), [201, 409]);
    assert_eq!(code_of(&answers, 409).as_deref(), Some("epoch-stale"));
    let stored: i64 = crowd
        .server
        .database()
        .query_row(
            "SELECT COUNT(*) FROM \"EncryptionEpochs\" WHERE \"ConversationId\" = ?1",
            [Id::parse(&conversation).unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, 1);
    let won: Value = answers.iter().find(|a| a.status == 201).unwrap().json();
    assert_eq!(won["sequence"], 1);
}

#[tokio::test]
async fn the_same_epoch_sent_twice_at_once_is_made_once_and_answered_twice() {
    let crowd = Crowd::ready().await;
    let conversation = crowd.conversation(&crowd.maja, &crowd.ada);
    crowd.enable(&crowd.maja, &crowd.ada, &conversation).await;
    let every = crowd.recipients(&crowd.maja.client, &conversation).await;
    let client = Arc::new(crowd.server.sign_in(&crowd.maja).await);
    let body = epoch_body(&Id::random().to_string(), None, &every);
    let requests: Vec<_> = (0..2)
        .map(|_| {
            let (client, conversation, body) = (Arc::clone(&client), conversation.clone(), body.clone());
            async move { post_epoch(&client, &conversation, &body).await }
        })
        .collect();

    let answers = together(requests).await;

    assert_eq!(statuses(&answers), [200, 201]);
}

#[tokio::test]
async fn two_attempts_at_once_when_one_is_left_delete_the_backup_once() {
    let crowd = Crowd::new().await;
    set_up(&crowd.maja.client).await;
    crowd
        .server
        .database()
        .execute(
            "UPDATE \"ProtectedAccountKeyBackups\" SET \"FailedAttempts\" = 9 WHERE \"UserId\" = ?1",
            params![Id::parse(&crowd.maja.id).unwrap()],
        )
        .unwrap();
    let client: Arc<Client> = Arc::new(crowd.server.sign_in(&crowd.maja).await);
    let requests: Vec<_> = (0..2)
        .map(|_| {
            let client = Arc::clone(&client);
            async move { post(&client, "/api/me/encryption/backup/unlock", &json!({ "version": 2, "authKey": text(&bytes(32, 9)) })).await }
        })
        .collect();

    let answers = together(requests).await;

    assert_eq!(statuses(&answers), [404, 410]);
    let left: i64 = crowd.server.database().query_row("SELECT COUNT(*) FROM \"ProtectedAccountKeyBackups\"", [], |row| row.get(0)).unwrap();
    assert_eq!(left, 0);
}
