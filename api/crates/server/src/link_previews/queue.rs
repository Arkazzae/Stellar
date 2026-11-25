//! The messages whose links are waiting for a card.

use std::collections::VecDeque;
use std::sync::Mutex;

use tokio::sync::Notify;

use crate::values::Id;

/// How many messages wait at most. A preview is decoration, so a flood may
/// cost cards but never a send and never the process's memory.
const CAPACITY: usize = 256;
const IMPORTED_CAPACITY: usize = 48;

/// A message whose links are waiting to be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewRequest {
    pub channel: Id,
    pub message: Id,
}

/// Hands messages from the request that stored them to the worker that reads
/// the pages they link to, so that no send waits for a host we do not control.
///
/// Bounded: when it is full the oldest request is forgotten. It lives in
/// memory, so a restart forgets what was waiting; those messages stay without
/// a card until they are edited.
#[derive(Debug, Default)]
pub struct PreviewQueue {
    requests: Mutex<VecDeque<PreviewRequest>>,
    arrived: Notify,
    imported: Mutex<VecDeque<PreviewRequest>>,
    imported_arrived: Notify,
}

impl PreviewQueue {
    pub fn enqueue(&self, request: PreviewRequest) {
        {
            let mut requests = self
                .requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if requests.len() >= CAPACITY {
                requests.pop_front();
            }
            requests.push_back(request);
        }
        // A permit is kept when nobody waits yet, so a request that arrives
        // between the worker's look and its wait is not missed.
        self.arrived.notify_one();
    }

    /// The oldest waiting request, once there is one.
    pub async fn next(&self) -> PreviewRequest {
        loop {
            let taken = self
                .requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front();
            if let Some(request) = taken {
                return request;
            }
            self.arrived.notified().await;
        }
    }

    pub(super) async fn next_imported(&self) -> PreviewRequest {
        loop {
            let taken = self
                .imported
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front();
            if let Some(request) = taken {
                return request;
            }
            self.imported_arrived.notified().await;
        }
    }

    pub(super) fn enqueue_imported(&self, request: PreviewRequest) {
        {
            let mut requests = self
                .imported
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if requests.len() >= IMPORTED_CAPACITY || requests.contains(&request) {
                return;
            }
            requests.push_back(request);
        }
        self.imported_arrived.notify_one();
    }

    pub fn len(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> PreviewRequest {
        PreviewRequest {
            channel: Id::random(),
            message: Id::random(),
        }
    }

    #[tokio::test]
    async fn requests_come_out_in_the_order_they_went_in() {
        let queue = PreviewQueue::default();
        let (first, second) = (request(), request());
        queue.enqueue(first);
        queue.enqueue(second);

        assert_eq!(queue.next().await, first);
        assert_eq!(queue.next().await, second);
    }

    #[tokio::test]
    async fn a_full_queue_forgets_the_oldest() {
        let queue = PreviewQueue::default();
        let oldest = request();
        queue.enqueue(oldest);
        let rest: Vec<PreviewRequest> = (0..CAPACITY).map(|_| request()).collect();
        for item in &rest {
            queue.enqueue(*item);
        }

        assert_eq!(queue.len(), CAPACITY);
        assert_eq!(queue.next().await, rest[0]);
    }

    #[tokio::test]
    async fn a_waiting_reader_is_woken_by_a_request() {
        let queue = std::sync::Arc::new(PreviewQueue::default());
        let reader = tokio::spawn({
            let queue = queue.clone();
            async move { queue.next().await }
        });
        tokio::task::yield_now().await;
        let sent = request();

        queue.enqueue(sent);

        assert_eq!(reader.await.unwrap(), sent);
    }
}
