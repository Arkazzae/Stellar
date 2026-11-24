//! Whether text is an address mail can be sent from, as the first server's
//! runtime read one: `user@host`, with anything before it as a name, in angle
//! brackets or not, and comments in parentheses almost anywhere.
//!
//! There is one reader of such addresses, `cockpit::address`, which was
//! checked against that runtime on half a million texts; this is the question
//! the settings ask of it.

/// Whether `text`, once trimmed, is an address with or without a name.
pub fn is_sender_address(text: &str) -> bool {
    crate::cockpit::address::try_create(text.trim()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_first_server_took_for_an_address_is_one() {
        for text in [
            "a@example.test",
            "a@b",
            "a.@example.test",
            "a@example.test.",
            "\"a b\"@example.test",
            "\"a\"b@example.test",
            "a b@example.test",
            "Name <a@example.test>",
            "Name<a@example.test>",
            "\"Na, me\" <a@example.test>",
            "Na, me <a@example.test>",
            "Na(me) <a@example.test>",
            "(comment) a@example.test",
            "a@example.test (comment)",
            "a(comment)@example.test",
            "a@(comment)example.test",
            "Name a@example.test",
            "<a@example.test>",
            "< a@example.test >",
            "Name <b@x.test> <a@example.test>",
            "a@example.test, b@example.test",
            "a@example.test b@example.test",
            "a@[1.2.3.4]",
            "\"Stellarr\" no-reply@example.test",
            "Stel@larr <a@example.test>",
            "zażółć@przykład.test",
            "Zażółć <a@example.test>",
        ] {
            assert!(is_sender_address(text), "{text}");
        }
    }

    #[test]
    fn what_it_refused_is_refused() {
        for text in [
            "",
            "a",
            "a@",
            "@",
            "@example.test",
            ".a@example.test",
            "a..b@example.test",
            "a@.example.test",
            "a@example..test",
            "a@exam ple.test",
            "a@@example.test",
            "a@b@example.test",
            "Na\"me <a@example.test>",
            "Name a@example.test>",
            "Name <a@example.test",
            "<a@example.test> Name",
            "<<a@example.test>>",
            "a@example.test<",
            "a@[1.2.3.4",
            "a@ex:ample.test",
            "no-reply@example.test Stellarr",
        ] {
            assert!(!is_sender_address(text), "{text}");
        }
    }

    #[test]
    fn an_unbalanced_comment_is_refused_and_never_a_fault() {
        for text in ["(", ")", "a@b(", "(a@b", "a@b)", "((", "a@b.test (x"] {
            assert!(!is_sender_address(text), "{text}");
        }
    }
}
