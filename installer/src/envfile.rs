//! `.env` as Compose reads it, and the installer's changes to it. The file is
//! handled as bytes: a line the installer does not change is written back
//! exactly as it was, whatever its encoding or line end.

use std::collections::BTreeMap;

fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

fn skip_blank(line: &[u8], mut at: usize) -> usize {
    while at < line.len() && is_blank(line[at]) {
        at += 1;
    }
    at
}

fn strip_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// The lines of a file: a last line without a line end counts, an empty end
/// after the last line end does not.
pub fn records(content: &[u8]) -> Vec<&[u8]> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&[u8]> = content.split(|&b| b == b'\n').collect();
    if content.ends_with(b"\n") {
        lines.pop();
    }
    lines
}

fn name_then_equals(line: &[u8], start: usize) -> Option<(String, usize)> {
    let first = *line.get(start)?;
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut end = start + 1;
    while end < line.len() && (line[end].is_ascii_alphanumeric() || line[end] == b'_') {
        end += 1;
    }
    let equals = skip_blank(line, end);
    if line.get(equals) == Some(&b'=') {
        Some((
            String::from_utf8_lossy(&line[start..end]).into_owned(),
            equals + 1,
        ))
    } else {
        None
    }
}

/// The name a line assigns, with blanks around it and an optional `export`
/// in front, and where its value starts.
pub fn assignment(line: &[u8]) -> Option<(String, usize)> {
    let line = strip_cr(line);
    let start = skip_blank(line, 0);
    if line[start..].starts_with(b"export") {
        let after = start + 6;
        let name = skip_blank(line, after);
        if name > after
            && let Some(found) = name_then_equals(line, name)
        {
            return Some(found);
        }
    }
    name_then_equals(line, start)
}

/// The value of an assignment, unquoted: a quoted one ends at its closing
/// quote, an unquoted one at a comment.
pub fn value(line: &[u8], at: usize) -> String {
    let line = strip_cr(line);
    let mut value = &line[skip_blank(line, at)..];
    match value.first() {
        Some(&quote @ (b'"' | b'\'')) => {
            value = &value[1..];
            if let Some(end) = value.iter().position(|&b| b == quote) {
                value = &value[..end];
            }
        }
        _ => {
            let mut i = 0;
            while i < value.len() {
                if is_blank(value[i]) {
                    let next = skip_blank(value, i);
                    if value.get(next) == Some(&b'#') {
                        value = &value[..i];
                        break;
                    }
                    i = next;
                } else {
                    i += 1;
                }
            }
            while value.last().is_some_and(|&b| is_blank(b)) {
                value = &value[..value.len() - 1];
            }
        }
    }
    String::from_utf8_lossy(value).into_owned()
}

/// Every key the file assigns and its value; the last assignment of a key
/// wins, as in Compose.
pub fn parse(content: &[u8]) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for line in records(content) {
        if let Some((name, at)) = assignment(line) {
            values.insert(name, value(line, at));
        }
    }
    values
}

/// The file with `marker` as its new first line.
pub fn with_marker(content: &[u8], marker: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + marker.len() + 2);
    out.extend_from_slice(marker.as_bytes());
    out.push(b'\n');
    for line in records(content) {
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    out
}

/// The file with each `(key, value)` set: the effective assignment of a key is
/// rewritten in place, keeping its line end, and a key the file lacks goes
/// under the first line. An empty COMPOSE_FILE removes every assignment of it.
pub fn with_values(content: &[u8], pairs: &[(String, String)]) -> Vec<u8> {
    let lines = records(content);
    let mut last: BTreeMap<String, usize> = BTreeMap::new();
    let mut remove = vec![false; lines.len()];
    let clear_compose_file = pairs
        .iter()
        .any(|(key, value)| key == "COMPOSE_FILE" && value.is_empty());
    for (row, line) in lines.iter().enumerate() {
        if let Some((name, _)) = assignment(line) {
            if name == "COMPOSE_FILE" && clear_compose_file {
                remove[row] = true;
            }
            last.insert(name, row);
        }
    }
    let mut at: BTreeMap<usize, &(String, String)> = BTreeMap::new();
    for pair in pairs {
        if let Some(&row) = last.get(&pair.0) {
            at.insert(row, pair);
        }
    }
    let missing: Vec<&(String, String)> = pairs
        .iter()
        .filter(|(key, _)| !last.contains_key(key))
        .collect();
    let write_pair = |out: &mut Vec<u8>, (key, value): &(String, String), cr: bool| {
        out.extend_from_slice(key.as_bytes());
        out.push(b'=');
        out.extend_from_slice(value.as_bytes());
        if cr {
            out.push(b'\r');
        }
        out.push(b'\n');
    };

    let mut out = Vec::with_capacity(content.len() + 64);
    for (row, line) in lines.iter().enumerate() {
        if !remove[row] {
            match at.get(&row) {
                Some(pair) => write_pair(&mut out, pair, line.ends_with(b"\r")),
                None => {
                    out.extend_from_slice(line);
                    out.push(b'\n');
                }
            }
        }
        if row == 0 {
            for pair in &missing {
                write_pair(&mut out, pair, false);
            }
        }
    }
    if lines.is_empty() {
        for pair in &missing {
            write_pair(&mut out, pair, false);
        }
    }
    out
}

/// `.env.example` with each assignment of a key in `ours` turned into a
/// comment, as it would override the installer's own setting above it.
pub fn commented(example: &[u8], ours: &[&str]) -> Vec<u8> {
    let mut out = Vec::with_capacity(example.len() + 64);
    for line in records(example) {
        if let Some((name, _)) = assignment(line)
            && ours.contains(&name.as_str())
        {
            out.extend_from_slice(b"# ");
        }
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn reads_values_as_compose_does() {
        let env = b"# comment\nSTELLARR_DOMAIN=\"old.example.com\"\nSTELLARR_DOMAIN='chat.example.com'\r\n\
export STELLARR_HTTPS_PORT=9443 # moved\nSTELLARR_SMTP_PASSWORD=\"pa ss # word\"\n  SPACED = value  \nexport=1\n\
STELLARR_LANGUAGE=en";
        let values = parse(env);
        assert_eq!(values["STELLARR_DOMAIN"], "chat.example.com");
        assert_eq!(values["STELLARR_HTTPS_PORT"], "9443");
        assert_eq!(values["STELLARR_SMTP_PASSWORD"], "pa ss # word");
        assert_eq!(values["SPACED"], "value");
        assert_eq!(values["export"], "1");
        assert_eq!(values["STELLARR_LANGUAGE"], "en");
        assert!(!values.contains_key("comment"));
    }

    #[test]
    fn unquoted_value_keeps_a_hash_without_blank() {
        assert_eq!(parse(b"KEY=a#b c # d\n")["KEY"], "a#b c");
        assert_eq!(parse(b"KEY=\"unterminated\n")["KEY"], "unterminated");
    }

    #[test]
    fn sets_the_effective_assignment_in_place() {
        let env = b"# marker\r\nSTELLARR_VERSION=1.3.0\r\nSTELLARR_VERSION=1.4.0\r\nOTHER=1\r\n";
        let out = with_values(
            env,
            &pairs(&[
                ("STELLARR_VERSION", "1.5.0"),
                ("STELLARR_DOMAIN", "x.example"),
            ]),
        );
        assert_eq!(
            out,
            b"# marker\r\nSTELLARR_DOMAIN=x.example\nSTELLARR_VERSION=1.3.0\r\nSTELLARR_VERSION=1.5.0\r\nOTHER=1\r\n"
        );
    }

    #[test]
    fn an_empty_compose_file_is_removed() {
        let env = b"# marker\nCOMPOSE_FILE=a.yml\nKEEP=1\nCOMPOSE_FILE=b.yml\n";
        assert_eq!(
            with_values(env, &pairs(&[("COMPOSE_FILE", "")])),
            b"# marker\nKEEP=1\n"
        );
    }

    #[test]
    fn adds_a_final_line_end() {
        assert_eq!(
            with_values(b"# m\nA=1", &pairs(&[("A", "2")])),
            b"# m\nA=2\n"
        );
        assert_eq!(with_values(b"", &pairs(&[("A", "2")])), b"A=2\n");
        assert_eq!(with_marker(b"", "# m"), b"# m\n");
        assert_eq!(with_marker(b"A=1", "# m"), b"# m\nA=1\n");
    }

    #[test]
    fn comments_out_our_keys() {
        let example = b"# STELLARR_DOMAIN=\nSTELLARR_DOMAIN=x\nSTELLARR_GIPHY_API_KEY=\r\n";
        assert_eq!(
            commented(example, &["STELLARR_DOMAIN"]),
            b"# STELLARR_DOMAIN=\n# STELLARR_DOMAIN=x\nSTELLARR_GIPHY_API_KEY=\r\n"
        );
    }
}
