use std::sync::atomic::{AtomicU64, Ordering};

/// Counts the responses the API sends, for the operator panel's runtime page.
#[derive(Debug, Default)]
pub struct Metrics {
    requests: AtomicU64,
    client_errors: AtomicU64,
    server_errors: AtomicU64,
}

impl Metrics {
    /// Records one finished response by its status code.
    pub fn count_response(&self, status: u16) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        if status >= 500 {
            self.server_errors.fetch_add(1, Ordering::Relaxed);
        } else if status >= 400 {
            self.client_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    pub fn client_errors(&self) -> u64 {
        self.client_errors.load(Ordering::Relaxed)
    }

    pub fn server_errors(&self) -> u64 {
        self.server_errors.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_are_counted_by_the_class_of_their_status() {
        let metrics = Metrics::default();

        for status in [200, 201, 404, 429, 500, 503] {
            metrics.count_response(status);
        }

        assert_eq!(
            (
                metrics.requests(),
                metrics.client_errors(),
                metrics.server_errors()
            ),
            (6, 2, 2)
        );
    }
}
