use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::to_bytes;
use rusqlite::params;
use serde_json::{Value, json};

use super::*;
use crate::database::{Database, DbError};
use crate::events::Unconnected;
use crate::seal::Sealer;
use crate::settings::Settings;
use crate::testing::TempDir;

#[path = "../../../discord/tests/support/mod.rs"]
mod discord;
use discord::guild::{Ch, FakeGuild, ID, Msg};
use discord::{Answer, Rig, TOKEN};

async fn setup() -> (TempDir, AppState, Caller) {
    let root = TempDir::new();
    let settings = Settings::load(&[
        ("STELLARR_ENVIRONMENT".into(), "Testing".into()),
        (
            "STELLARR_DATA_ROOT".into(),
            root.path().display().to_string(),
        ),
    ])
    .unwrap();
    std::fs::create_dir_all(settings.database.path.parent().unwrap()).unwrap();
    let database = Database::open(&settings.database).unwrap();
    let actor = Id::random();
    database
        .write(move |db| -> Result<(), DbError> {
            db.execute(
            "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\",
                                  \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\",
                                  \"AvatarColor\", \"CreatedAt\")
             VALUES (?1, 'owner', 'OWNER', 'Owner', 0, 0, 0, '#000', ?2)",
            params![actor, Timestamp::now()],
        )?;
            Ok(())
        })
        .await
        .unwrap();
    let state = AppState::new(
        settings,
        database,
        Arc::new(Unconnected),
        Arc::new(Unconnected),
        Sealer::from_key([5; 32]),
    );
    (
        root,
        state,
        Caller {
            user: actor,
            session: Id::random(),
        },
    )
}

async fn outcome(state: &AppState, caller: Caller, id: Id, expected: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let Json(job) = status(State(state.clone()), caller, Path(id.to_string()))
                .await
                .unwrap();
            if job.state == expected {
                return serde_json::to_value(job).unwrap();
            }
            assert_eq!(job.state, "running");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap()
}

fn scope(channels: &[&str]) -> Options {
    Options {
        source_id: ID.to_owned(),
        name: "Imported Discord".into(),
        channel_ids: channels.iter().map(|id| (*id).to_owned()).collect(),
        history: History::Since {
            since: Timestamp::parse("2024-01-01T00:02:00Z").unwrap(),
        },
        include_threads: true,
    }
}

#[tokio::test]
async fn the_wizard_enforces_the_preview_and_resumes_the_selected_history_on_the_same_server() {
    let guild = FakeGuild::new("Source");
    guild
        .category("90", "Discussion", 0)
        .channel(
            "100",
            "lobby",
            Ch {
                parent: Some("90"),
                ..Ch::default()
            },
        )
        .text("101", "unselected")
        .channel(
            "102",
            "staff",
            Ch {
                hidden: true,
                ..Ch::default()
            },
        )
        .text("103", "restricted-history")
        .thread("210", "100", "public thread", 11)
        .thread("211", "100", "private thread", 12)
        .message("1001", "100", "900", "before the cutoff", 1, Msg::default())
        .message(
            "1002",
            "100",
            "900",
            "the selected message <@901> https://klipy.com/gifs/wave",
            2,
            Msg {
                mention: Some(("901", "ula")),
                embed_link: Some("https://klipy.com/gifs/wave"),
                attachments: 1,
                ..Msg::default()
            },
        )
        .message(
            "1003",
            "100",
            "905",
            "",
            3,
            Msg {
                attachments: 1,
                ..Msg::default()
            },
        )
        .message(
            "2100",
            "210",
            "900",
            "public thread message",
            3,
            Msg::default(),
        )
        .message(
            "2110",
            "211",
            "900",
            "private thread message",
            3,
            Msg::default(),
        );
    let fail_first_thread_page = AtomicBool::new(true);
    let respond = guild.handler();
    let rig = Rig::new(move |request, n| {
        let path = request.path();
        match path {
            "/api/v10/users/@me" => return Answer::json(r#"{"id":"5","username":"import","bot":true}"#),
            "/api/v10/users/@me/guilds" => return Answer::json(json!([{"id":ID,"name":"Source"}]).to_string()),
            "/api/v10/applications/@me" => return Answer::json(r#"{"id":"5","flags":524288}"#),
            "/api/v10/channels/210/messages" if fail_first_thread_page.swap(false, Ordering::SeqCst) => {
                return Answer::error(403, 50013, "Missing Permissions");
            }
            _ => {}
        }
        if path.ends_with("/roles") {
            return Answer::json(json!([{"id":ID,"name":"@everyone","permissions":"66560"}]).to_string());
        }
        if path.ends_with("/channels") {
            // An explicit denial for someone other than @everyone matters
            // too, even when everyone else's Discord role can read history.
            return Answer::json(json!([
                {"id":"90","name":"Discussion","type":4},
                {"id":"100","name":"lobby","type":0,"parent_id":"90"},
                {"id":"101","name":"unselected","type":0},
                {"id":"102","name":"staff","type":0,"permission_overwrites":[{"id":ID,"type":0,"allow":"0","deny":"1024"}]},
                {"id":"103","name":"restricted-history","type":0,"permission_overwrites":[{"id":"900","type":1,"allow":"0","deny":"65536"}]}
            ]).to_string());
        }
        respond(request, n)
    }).await;
    let (_root, state, actor) = setup().await;
    let state = state.with_discord_client(rig.client.clone());
    let credentials = || Credentials {
        bot_token: TOKEN.to_owned(),
    };

    let Json(discovered) = servers(State(state.clone()), actor, Json(credentials()))
        .await
        .unwrap();
    assert_eq!(discovered.servers[0].id, ID);
    assert_eq!(
        discovered.install_url,
        "https://discord.com/oauth2/authorize?client_id=5&scope=bot&permissions=66560&integration_type=0"
    );
    let Json(preview) = preview(
        State(state.clone()),
        actor,
        Json(PreviewRequest {
            source_id: ID.into(),
            bot_token: TOKEN.into(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(preview.restricted_channels, 1);
    assert!(
        !preview
            .channels
            .iter()
            .find(|channel| channel.id == "103")
            .unwrap()
            .can_copy_history
    );
    assert!(preview.message_content);

    let request = |id, channels: &[&str]| {
        Json(StartRequest {
            id,
            bot_token: TOKEN.into(),
            options: scope(channels),
        })
    };
    for channel in ["102", "103"] {
        assert!(
            start(
                State(state.clone()),
                actor,
                request(Id::random(), &[channel])
            )
            .await
            .is_err()
        );
    }
    let id = Id::random();
    let response = start(State(state.clone()), actor, request(id, &["100"]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let response: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert!(response["serverId"].is_null());
    let failure = outcome(&state, actor, id, "failed").await;
    assert_eq!(failure["error"], "discord-bot-access");
    assert_eq!(failure["completedSources"], 1);
    assert!(failure["serverId"].is_null());
    let owner = actor.user;
    let members: i64 = state
        .database()
        .read(move |db| -> Result<i64, DbError> {
            Ok(db.query_row(
                "SELECT COUNT(*) FROM \"ServerMembers\" WHERE \"UserId\" = ?1",
                [owner],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(members, 0);
    let stranger = Caller {
        user: Id::random(),
        session: Id::random(),
    };
    assert!(
        status(State(state.clone()), stranger, Path(id.to_string()))
            .await
            .is_err()
    );

    resume(
        State(state.clone()),
        actor,
        Path(id.to_string()),
        Json(credentials()),
    )
    .await
    .unwrap();
    let result = outcome(&state, actor, id, "completed").await;
    assert_eq!(result["report"]["messages"], 2);
    assert_eq!(result["pendingAttachments"], 2);
    assert_eq!(result["channelsImported"], 1);
    assert_eq!(result["sectionsImported"], 1);
    assert_eq!(result["historyRequested"], true);
    let server = Id::parse(result["serverId"].as_str().unwrap()).unwrap();
    assert_eq!(result["completedSources"], 2);

    let recorded: Vec<String> = state.database().read(move |db| -> Result<Vec<String>, DbError> {
        let mut sql = db.prepare(
            "SELECT \"DiscordId\" FROM \"DiscordImportMessages\" WHERE \"ServerId\" = ?1 ORDER BY \"DiscordId\"",
        )?;
        Ok(sql.query_map([server], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?)
    }).await.unwrap();
    assert_eq!(recorded, ["1002", "1003", "2100"]);
    let (content, cards, attempted) = state.database().read(move |db| -> Result<_, DbError> {
        Ok(db.query_row(
            "SELECT m.\"Content\", p.\"Cards\", p.\"AttemptedAt\" FROM \"DiscordImportMessages\" i
             JOIN \"Messages\" m ON m.\"Id\" = i.\"MessageId\"
             JOIN \"DiscordImportPreviews\" p ON p.\"MessageId\" = m.\"Id\"
             WHERE i.\"ServerId\" = ?1 AND i.\"DiscordId\" = '1002'",
            [server],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<Timestamp>>(2)?)),
        )?)
    }).await.unwrap();
    assert_eq!(
        content,
        r"the selected message \@Ula https://klipy.com/gifs/wave"
    );
    let cards: Value = serde_json::from_str(&cards).unwrap();
    assert_eq!(cards[0]["title"], "A wave");
    assert_eq!(
        cards[0]["image_url"],
        "https://images-ext-1.discordapp.net/external/wave.gif"
    );
    assert_eq!(
        cards[0]["animation_url"],
        "https://static.klipy.com/ii/wave.mp4"
    );
    assert!(attempted.is_none());
    assert!(!rig.server.targets().iter().any(|path| {
        ["101", "102", "103", "211"]
            .iter()
            .any(|channel| path.contains(&format!("/channels/{channel}/")))
    }));
    let Json(recent) = latest(State(state.clone()), actor).await.unwrap();
    assert_eq!(
        serde_json::to_value(recent.unwrap()).unwrap()["id"],
        result["id"]
    );
    let published = Jobs::find(&state, id, actor.user).await.unwrap().unwrap();
    let imported_channel: Id = state.database().read(move |db| -> Result<_, DbError> {
        Ok(db.query_row(
            "SELECT \"ChannelId\" FROM \"DiscordImportChannels\" WHERE \"ServerId\" = ?1 AND \"DiscordId\" = '100'",
            [server], |row| row.get(0),
        )?)
    }).await.unwrap();
    let originals = rig
        .bot()
        .messages("100", None)
        .unwrap()
        .collect_all()
        .await
        .unwrap();
    let writer = crate::discord_import::ImportWriter::new(state.database().clone());
    let mut ids = Vec::new();
    for (discord_id, content_type, name) in [
        ("1002", "image/png", "photo.png"),
        ("1003", "video/mp4", "clip.mp4"),
    ] {
        let message = originals
            .iter()
            .find(|message| message.id == discord_id)
            .unwrap();
        let destination = crate::discord_import::names::message_id(server, discord_id);
        ids.push(destination);
        let asset = crate::uploads::Attachment {
            id: Id::random(),
            uploaded_by: actor.user,
            object_key: format!("import-test/{name}"),
            file_name: name.into(),
            content_type: content_type.into(),
            size: 1024,
            width: Some(640),
            height: Some(480),
            duration_seconds: None,
            is_voice_message: false,
            poster: None,
            preview: None,
            placeholder: None,
            is_encrypted: false,
        };
        assert!(
            writer
                .attach(&published.import, imported_channel, message, asset)
                .await
                .unwrap()
        );
    }
    let messages = *state.messages();
    let recovered = state
        .database()
        .read(move |db| crate::messages::dto::load(db, &messages, &ids, Some(actor.user)))
        .await
        .unwrap();
    assert_eq!(recovered[0].content, content);
    assert_eq!(recovered[0].attachments[0].content_type, "image/png");
    assert!(recovered[1].content.is_empty());
    assert_eq!(recovered[1].attachments[0].content_type, "video/mp4");
    assert_eq!(writer.tally(server).await.unwrap().written, 3);

    let replay = start(State(state), actor, request(id, &["100"]))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
}
