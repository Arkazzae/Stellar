//! Group only already-queued posts, keeping separate outcomes for their callers.
use super::{Posted, Submission, post};
use crate::database::{Database, DbError};
use crate::http::ApiError;
use crate::messages::Stop;
use crate::values::Id;
use std::sync::{Arc, OnceLock};
use stellarr_messages::Messages;
use tokio::sync::{Mutex, mpsc, oneshot};

#[cfg(test)]
mod tests;

const BATCH_LIMIT: usize = 4;
const QUEUE_CAPACITY: usize = 128;

type Outcome = Result<Posted, Stop>;

struct Job {
    server: Id,
    submission: Submission,
    reply: oneshot::Sender<Outcome>,
}

#[derive(Default)]
pub(crate) struct PostQueue {
    sender: OnceLock<mpsc::Sender<Job>>,
    turn: Arc<Mutex<()>>,
}

impl PostQueue {
    pub(crate) async fn submit(
        &self,
        database: &Database,
        messages: Messages,
        server: Id,
        submission: Submission,
    ) -> Outcome {
        if let Ok(_turn) = self.turn.try_lock() {
            return database
                .write(move |tx| post(tx, &messages, server, &submission))
                .await;
        }

        let sender = self.sender.get_or_init(|| {
            let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
            tokio::spawn(run(
                database.clone(),
                messages,
                receiver,
                Arc::clone(&self.turn),
            ));
            sender
        });
        let (reply, answer) = oneshot::channel();
        sender
            .send(Job {
                server,
                submission,
                reply,
            })
            .await
            .map_err(|_| ApiError::internal("the message writer has stopped"))?;
        answer
            .await
            .map_err(|_| ApiError::internal("the message writer dropped the result"))?
    }
}

async fn run(
    database: Database,
    messages: Messages,
    mut receiver: mpsc::Receiver<Job>,
    turn: Arc<Mutex<()>>,
) {
    while let Some(first) = receiver.recv().await {
        let _turn = turn.lock().await;
        let Ok(second) = receiver.try_recv() else {
            let Job {
                server,
                submission,
                reply,
            } = first;
            let result = database
                .write(move |tx| post(tx, &messages, server, &submission))
                .await;
            let _ = reply.send(result);
            continue;
        };

        let mut jobs = Vec::with_capacity(BATCH_LIMIT);
        jobs.extend([first, second]);
        while jobs.len() < BATCH_LIMIT {
            match receiver.try_recv() {
                Ok(job) => jobs.push(job),
                Err(_) => break,
            }
        }
        let (requests, replies): (Vec<_>, Vec<_>) = jobs
            .into_iter()
            .map(|job| ((job.server, job.submission), job.reply))
            .unzip();

        let result = database
            .write(move |tx| -> Result<_, DbError> {
                let mut outcomes = Vec::with_capacity(requests.len());
                for (server, submission) in requests {
                    // A later step can reject a post after its INSERT succeeded.
                    tx.execute_batch("SAVEPOINT message_post")?;
                    let outcome = post(tx, &messages, server, &submission);
                    if outcome.is_err() {
                        tx.execute_batch("ROLLBACK TO message_post")?;
                    }
                    tx.execute_batch("RELEASE message_post")?;
                    outcomes.push(outcome);
                }
                Ok(outcomes)
            })
            .await;

        match result {
            Ok(outcomes) => {
                for (reply, outcome) in replies.into_iter().zip(outcomes) {
                    let _ = reply.send(outcome);
                }
            }
            Err(error) => {
                let message = error.to_string();
                for reply in replies {
                    let _ = reply.send(Err(ApiError::internal(message.clone()).into()));
                }
            }
        }
    }
}
