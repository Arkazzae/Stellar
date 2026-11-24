//! The query of the inbox's list, read as the first server read it. Kept apart
//! so that it can be replaced by the shared reader of query values.

/// `limit` and `beforeSequence` from the query, read as the first server read
/// them: names in any case, values given twice joined with a comma, a number
/// with the spaces and sign a number may have. A name that is there with a
/// value that is no number, or with none, does not bind.
pub struct PageQuery {
    pub limit: Option<i32>,
    pub before_sequence: Option<i64>,
}

fn is_blank(c: char) -> bool {
    matches!(c, '\t'..='\r' | ' ')
}

fn number<T: std::str::FromStr>(text: &str) -> Result<T, ()> {
    let trimmed = text.trim_matches(is_blank);
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    trimmed.parse::<T>().map_err(|_| ())
}

fn bound<T: std::str::FromStr>(pairs: &[(String, String)], name: &str) -> Result<Option<T>, ()> {
    let values: Vec<&str> = pairs
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
        .collect();
    if values.is_empty() {
        return Ok(None);
    }
    number(&values.join(",")).map(Some)
}

impl PageQuery {
    pub fn read(pairs: &[(String, String)]) -> Result<PageQuery, ()> {
        Ok(PageQuery {
            limit: bound(pairs, "limit")?,
            before_sequence: bound(pairs, "beforeSequence")?,
        })
    }
}
