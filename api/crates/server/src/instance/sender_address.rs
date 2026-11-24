//! Whether text is an address mail can be sent from, as the first server's
//! runtime read one: `user@host`, with anything before it as a name, in angle
//! brackets or not (`Stellarr <a@b.test>`, `"Stellarr" a@b.test`), and
//! comments in parentheses almost anywhere.
//!
//! The text is read from its end, as that runtime read it: the domain, then the
//! `@`, then the local part, and whatever is left in front is the name. That is
//! why a name needs no brackets, why `a@b.test, c@d.test` is the address
//! `c@d.test` with the name `a@b.test,`, and why a name may hold `@` and `.`.
//! What a name may not hold is a quote that is not a whole quoted string.
//!
//! A local part is atoms separated by single dots (one dot may end it), or a
//! quoted string; a domain is atoms separated by single dots (one dot may end
//! it) or a literal in square brackets. An atom is letters and digits of any
//! script and ``!#$%&'*+-/=?^_`{|}~``.

const SYMBOLS: &str = "!#$%&'*+-/=?^_`{|}~";

fn is_atom_char(c: char) -> bool {
    stellarr_unicode::is_letter_or_digit(c) || SYMBOLS.contains(c)
}

/// The end of the text once whitespace and comments are taken off it.
fn trim_end(chars: &[char]) -> usize {
    let mut end = chars.len();
    loop {
        while end > 0 && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        if end > 0 && chars[end - 1] == ')' {
            match comment_start(chars, end) {
                Some(start) => end = start,
                None => return end,
            }
        } else {
            return end;
        }
    }
}

/// Where the comment that ends at `end` (just after its `)`) starts. Comments
/// nest and may hold quoted pairs.
fn comment_start(chars: &[char], end: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut at = end;
    while at > 0 {
        at -= 1;
        let escaped = at > 0 && chars[at - 1] == '\\';
        match chars[at] {
            ')' if !escaped => depth += 1,
            '(' if !escaped => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

/// Whether `domain` is a run of atoms separated by single dots, one dot
/// allowed at the end, or a literal.
fn is_domain(domain: &[char]) -> bool {
    let text: String = domain.iter().collect();
    if let Some(inner) = text
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        return !inner.is_empty() && !inner.contains(['[', ']', '\\', '\r', '\n']);
    }
    let text = text.strip_suffix('.').unwrap_or(&text);
    !text.is_empty()
        && text
            .split('.')
            .all(|atom| !atom.is_empty() && atom.chars().all(is_atom_char))
}

fn is_quoted_string(text: &str) -> bool {
    let Some(inner) = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return false;
    };
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.next().is_none() => return false,
            '"' => return false,
            _ => {}
        }
    }
    true
}

/// A name in front of the address: nothing, a quoted string, or text with no
/// quote in it.
fn is_name(name: &str) -> bool {
    let name = name.trim();
    name.is_empty() || is_quoted_string(name) || !name.contains('"')
}

/// Reads `local@domain` from the end of `chars` and says where its local part
/// starts, when it is one.
fn address_start(chars: &[char]) -> Option<usize> {
    let end = trim_end(chars);
    // The domain: a literal, or atoms and dots.
    let mut domain_at = end;
    if end > 0 && chars[end - 1] == ']' {
        domain_at = chars[..end].iter().rposition(|c| *c == '[')?;
    } else {
        while domain_at > 0 && (is_atom_char(chars[domain_at - 1]) || chars[domain_at - 1] == '.') {
            domain_at -= 1;
        }
    }
    if !is_domain(&chars[domain_at..end]) {
        return None;
    }
    // The `@`, which may have spaces and comments on either side of it.
    let before = trim_end(&chars[..domain_at]);
    if before == 0 || chars[before - 1] != '@' {
        return None;
    }
    let local_end = trim_end(&chars[..before - 1]);
    if local_end == 0 {
        return None;
    }
    // The local part: a quoted string, or atoms and dots.
    if chars[local_end - 1] == '"' {
        let mut start = local_end - 1;
        while start > 0 {
            start -= 1;
            if chars[start] == '"' && (start == 0 || chars[start - 1] != '\\') {
                return Some(start);
            }
        }
        return None;
    }
    let mut start = local_end;
    while start > 0 && (is_atom_char(chars[start - 1]) || chars[start - 1] == '.') {
        start -= 1;
    }
    let local: String = chars[start..local_end].iter().collect();
    (!local.is_empty() && !local.starts_with('.') && !local.contains("..")).then_some(start)
}

/// Whether `text`, once trimmed, is an address with or without a name.
pub fn is_sender_address(text: &str) -> bool {
    let chars: Vec<char> = text.trim().chars().collect();
    let end = trim_end(&chars);
    if end == 0 {
        return false;
    }
    if chars[end - 1] == '>' {
        let Some(open) = chars[..end - 1].iter().rposition(|c| *c == '<') else {
            return false;
        };
        let inside = &chars[open + 1..end - 1];
        let Some(start) = address_start(inside) else {
            return false;
        };
        let name: String = chars[..open].iter().collect();
        return inside[..start].iter().all(|c| c.is_whitespace()) && is_name(&name);
    }
    let Some(start) = address_start(&chars[..end]) else {
        return false;
    };
    let name: String = chars[..start].iter().collect();
    // A name that ends in `<` is a bracket that was never closed, and one that
    // ends in `@` is another address run into this one.
    !name.ends_with('@') && !name.trim_end().ends_with('<') && is_name(&name)
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
}
