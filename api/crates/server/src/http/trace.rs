//! The identifier of a request, which a failure report and the log both carry.
//!
//! It has the shape of a W3C trace context, `00-<32 hex>-<16 hex>-00`, because
//! that is what the clients have always been shown in a problem's `traceId`.

use std::future::Future;

use aws_lc_rs::rand;

tokio::task_local! {
    static TRACE: String;
}

/// A new identifier.
pub fn new_id() -> String {
    let mut bytes = [0u8; 24];
    rand::fill(&mut bytes).expect("the system random number generator failed");
    let trace = crate::encoding::hex_lower(&bytes[..16]);
    let span = crate::encoding::hex_lower(&bytes[16..]);
    format!("00-{trace}-{span}-00")
}

/// Runs `work` with `id` as the identifier of the request it serves.
pub async fn scope<F: Future>(id: String, work: F) -> F::Output {
    TRACE.scope(id, work).await
}

/// The identifier of the request being served, or none outside one: what the
/// log wants, which must not invent an identifier for a line that no request
/// wrote.
pub fn try_current() -> Option<String> {
    TRACE.try_with(Clone::clone).ok()
}

/// The identifier of the request being served. Outside one, a new identifier,
/// so that a problem made in a test or at start still has one.
pub fn current() -> String {
    TRACE.try_with(Clone::clone).unwrap_or_else(|_| new_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identifier_has_the_shape_of_a_trace_context() {
        let id = new_id();

        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!(
            (parts[0], parts[1].len(), parts[2].len(), parts[3]),
            ("00", 32, 16, "00")
        );
        assert!(
            id.bytes()
                .all(|b| b == b'-' || b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_ne!(id, new_id());
    }

    #[tokio::test]
    async fn a_request_keeps_its_identifier_to_the_end() {
        let seen = scope("00-a-b-00".to_string(), async { current() }).await;

        assert_eq!(seen, "00-a-b-00");
    }
}
