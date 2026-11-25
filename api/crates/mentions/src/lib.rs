//! Which accounts a message names with `@username`, read the way Stellarr
//! reads them when it decides whom to notify.
//!
//! What the web client does not show as a mention is not one here either:
//! fenced code, code spans, addresses, escaped characters and the labels of
//! real http(s) links stay quiet. The rewriting of a Discord message checks
//! its own result against this reader, and the server's message code decides
//! whom to notify with it, so there is one reader and no copy to drift. The
//! tests of this crate, with the cases shared with the web renderer, are its
//! specification.
//!
//! The body is read linearly: fences first, then the prose between them, then
//! the names outside the ranges that prose marks. No construct is searched
//! again from the start, so no part of a long message is read a number of
//! times that grows with its length.

use std::collections::{BTreeSet, HashMap};

use stellarr_unicode::utf16::{
    REPLACEMENT, is_ascii_punctuation, is_high_surrogate, is_js_whitespace, is_low_surrogate,
    scalar_at, scalar_before, text, to_upper_units, units,
};
use stellarr_unicode::{is_letter_or_digit, is_punctuation, is_symbol};

const MIN_NAME: usize = 2;
const MAX_NAME: usize = 32;

const BACKTICK: u16 = b'`' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const AT: u16 = b'@' as u16;

/// The upper-cased names mentioned in `content`. A name starts at an `@` that
/// no name character precedes, so an e-mail address is not one, and is the
/// whole word after it: a word longer than 32 characters names nobody, and
/// its first 32 characters must not name a shorter member either.
pub fn extract_normalized_usernames(content: &str) -> BTreeSet<String> {
    let source = units(content);
    extract_units(&source)
        .into_iter()
        .map(|name| text(&name))
        .collect()
}

pub fn extract_units(content: &[u16]) -> BTreeSet<Vec<u16>> {
    let mut names = BTreeSet::new();
    if content.is_empty() {
        return names;
    }

    let excluded = excluded_ranges(content);
    let mut index = 0;
    while index < content.len() {
        if content[index] != AT
            || excluded[index]
            || (index > 0 && name_character_before(content, index, &excluded))
        {
            index += 1;
            continue;
        }

        let mut end = index + 1;
        while end < content.len() && !excluded[end] {
            let width = name_character_width(content, end);
            if width == 0 {
                break;
            }
            end += width;
        }

        let length = end - index - 1;
        if (MIN_NAME..=MAX_NAME).contains(&length) {
            names.insert(to_upper_units(&content[index + 1..end]));
        }
        index = end;
    }
    names
}

/* ---------------------------------------------------------- exclusions */

/// What the renderer does not read as plain words: fenced blocks, code spans,
/// escaped characters, bare addresses and the labels of valid http(s) links.
/// A fenced block is cut out before anything inline is read, as it is in the
/// transcript, so no inline construct can span one.
fn excluded_ranges(content: &[u16]) -> Vec<bool> {
    let mut excluded = vec![false; content.len()];
    let mut from = 0;
    for (start, end) in fenced_ranges(content) {
        exclude_inline(&content[from..start], &mut excluded, from);
        exclude(&mut excluded, start, end);
        from = end;
    }
    exclude_inline(&content[from..], &mut excluded, from);
    excluded
}

#[derive(Clone, Copy)]
struct Bracket {
    position: usize,
    image: bool,
    floor: Option<usize>,
}

/// Reads one stretch of prose, marking everything in it the renderer does not
/// read as plain words.
fn exclude_inline(text: &[u16], excluded: &mut [bool], offset: usize) {
    if text.is_empty() {
        return;
    }

    let mut brackets: Vec<Bracket> = Vec::new();
    let mut runs = BacktickRuns::new(text);
    let parens = ClosingParentheses::new(text);
    let mut delimiters = Delimiters::new(text, offset);
    let addresses = protected_addresses(text);
    let mut address = 0;
    // The character after this offset was escaped and is not an image marker.
    let mut escaped_up_to: isize = -1;
    // No link may start before this offset: an inner link wins over an outer one.
    let mut inactive_before: isize = -1;

    let mut index = 0;
    while index < text.len() {
        while address < addresses.len() && addresses[address].0 < index {
            address += 1;
        }
        if address < addresses.len() && addresses[address].0 == index {
            index = addresses[address].1;
            continue;
        }

        let current = text[index];
        if current == BACKSLASH && index + 1 < text.len() && is_ascii_punctuation(text[index + 1]) {
            excluded[offset + index + 1] = true;
            escaped_up_to = (index + 2) as isize;
            index += 2;
            continue;
        }

        if current == BACKTICK {
            // A span closes only at a run of exactly the same length.
            let run = run_length(text, index, BACKTICK);
            match runs.next(run, index + run) {
                None => index += run,
                Some(close) => {
                    exclude(excluded, offset + index, offset + close + run);
                    index = close + run;
                }
            }
            continue;
        }

        if current == b'[' as u16 {
            brackets.push(Bracket {
                position: index,
                image: index > 0
                    && text[index - 1] == b'!' as u16
                    && escaped_up_to != index as isize,
                floor: delimiters.top,
            });
            index += 1;
            continue;
        }

        if current == b']' as u16 {
            let Some(opener) = brackets.pop() else {
                index += 1;
                continue;
            };
            let target = if !opener.image && opener.position as isize >= inactive_before {
                link_target(text, index + 1, &parens)
            } else {
                None
            };
            match target {
                Some(target_end) => {
                    delimiters.process(opener.floor, excluded);
                    exclude(excluded, offset + opener.position, offset + target_end);
                    inactive_before = opener.position as isize;
                    index = target_end;
                }
                None => index += 1,
            }
            continue;
        }

        if matches!(current, 0x2A | 0x5F | 0x7E | 0x7C) {
            let length = run_length(text, index, current);
            delimiters.add(index, current, length);
            index += length;
            continue;
        }

        index += 1;
    }

    delimiters.process(None, excluded);
    exclude_addresses(text, excluded, offset);
}

/// Addresses shield their markers from the grammar, but a closing bracket
/// must still be read as the end of a possible label. Actual URL exclusions
/// are applied after links and formatting have split the prose into spans.
fn protected_addresses(text: &[u16]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut index = 0;
    while index < text.len() {
        if text[index] == b'h' as u16
            && let Some((_, match_end)) = bare_address_end(text, index)
        {
            let bracket = text[index..match_end]
                .iter()
                .position(|&u| u == b']' as u16);
            let end = bracket.map_or(match_end, |at| index + at);
            let end = trim_address(text, index, end, true);
            if end > index {
                found.push((index, end));
            }
            index = match_end;
            continue;
        }
        index += 1;
    }
    found
}

fn exclude_addresses(text: &[u16], excluded: &mut [bool], offset: usize) {
    let mut index = 0;
    while index < text.len() {
        if excluded[offset + index] {
            index += 1;
            continue;
        }
        let mut end = index;
        while end < text.len() && !excluded[offset + end] {
            end += 1;
        }
        let plain = &text[index..end];
        let mut at = 0;
        while at < plain.len() {
            if plain[at] == b'h' as u16
                && let Some((address_end, match_end)) = bare_address_end(plain, at)
            {
                exclude(excluded, offset + index + at, offset + index + address_end);
                at = match_end;
                continue;
            }
            at += 1;
        }
        index = end;
    }
}

/// Only paired markers split the text a mention recogniser reads. In
/// particular, the last underscore in `_@ada_` closes emphasis, whereas the
/// one in `@ada_` belongs to that member's name. All marker kinds share the
/// stack so crossing pairs discard the same inner openers as the renderer.
struct Run {
    position: usize,
    character: u16,
    length: usize,
    opens: bool,
    closes: bool,
    count: usize,
    start: usize,
    end: usize,
    previous: Option<usize>,
    next: Option<usize>,
}

/// The runs live in a list that only grows and are linked by index. A run
/// that leaves the chain keeps the links it had, as the algorithm expects.
struct Delimiters<'a> {
    source: &'a [u16],
    offset: usize,
    runs: Vec<Run>,
    bottom: Option<usize>,
    top: Option<usize>,
    visible: Option<Vec<usize>>,
}

impl<'a> Delimiters<'a> {
    fn new(source: &'a [u16], offset: usize) -> Delimiters<'a> {
        Delimiters {
            source,
            offset,
            runs: Vec::new(),
            bottom: None,
            top: None,
            visible: None,
        }
    }

    fn add(&mut self, position: usize, character: u16, length: usize) {
        let before = scalar_before(self.source, position);
        let after = if position + length >= self.source.len() {
            None
        } else {
            Some(rune_at(self.source, position + length))
        };
        let before = if position == 0 {
            None
        } else {
            Some(before.unwrap_or(REPLACEMENT))
        };
        let left = !space(after) && (!punctuation(after) || space(before) || punctuation(before));
        let right = !space(before) && (!punctuation(before) || space(after) || punctuation(after));
        let (opens, closes) = match character {
            0x7C => (length >= 2, length >= 2),
            0x7E => (left && length == 2, right && length == 2),
            0x5F => (
                left && (!right || punctuation(before)),
                right && (!left || punctuation(after)),
            ),
            _ => (left, right),
        };
        if !opens && !closes {
            return;
        }
        let index = self.runs.len();
        self.runs.push(Run {
            position,
            character,
            length,
            opens,
            closes,
            count: length,
            start: position,
            end: position + length,
            previous: self.top,
            next: None,
        });
        match self.top {
            None => self.bottom = Some(index),
            Some(top) => self.runs[top].next = Some(index),
        }
        self.top = Some(index);
    }

    fn process(&mut self, floor: Option<usize>, excluded: &mut [bool]) {
        // A failed search is the lower bound for later closers of that kind,
        // including the rule-of-three remainder. Each run is visited or
        // removed a bounded number of times, even on hostile input.
        let mut searched: HashMap<(u16, bool, usize), isize> = HashMap::new();
        let floor_position = floor.map_or(-1, |f| self.runs[f].position as isize);
        let mut closer = match floor {
            None => self.bottom,
            Some(f) => self.runs[f].next,
        };
        while let Some(c) = closer {
            if !self.runs[c].closes {
                closer = self.runs[c].next;
                continue;
            }
            let character = self.runs[c].character;
            let emphasis = character == 0x2A || character == 0x5F;
            let kind = (
                character,
                emphasis && self.runs[c].opens,
                if emphasis { self.runs[c].length % 3 } else { 0 },
            );
            let lowest = floor_position.max(searched.get(&kind).copied().unwrap_or(-1));
            let mut opener = self.runs[c].previous;
            while let Some(o) = opener {
                if self.runs[o].position as isize <= lowest {
                    break;
                }
                if self.runs[o].character == character && self.runs[o].opens {
                    if self.pairs(o, c) {
                        break;
                    }
                    if character == 0x7C {
                        opener = None;
                        break;
                    }
                }
                opener = self.runs[o].previous;
            }
            if let Some(o) = opener
                && self.runs[o].position as isize <= lowest
            {
                opener = None;
            }
            let Some(o) = opener else {
                searched.insert(
                    kind,
                    self.runs[c]
                        .previous
                        .map_or(-1, |p| self.runs[p].position as isize),
                );
                let next = self.runs[c].next;
                if !self.runs[c].opens {
                    self.remove(c);
                }
                closer = next;
                continue;
            };

            let used = if emphasis && (self.runs[o].count < 2 || self.runs[c].count < 2) {
                1
            } else {
                2
            };
            self.runs[o].count = self.runs[o].count.saturating_sub(used);
            self.runs[o].end = self.runs[o].end.saturating_sub(used);
            let opener_end = self.runs[o].end;
            exclude(
                excluded,
                self.offset + opener_end,
                self.offset + opener_end + used,
            );
            let closer_start = self.runs[c].start;
            exclude(
                excluded,
                self.offset + closer_start,
                self.offset + closer_start + used,
            );
            self.runs[c].count = self.runs[c].count.saturating_sub(used);
            self.runs[c].start += used;
            self.runs[o].next = Some(c);
            self.runs[c].previous = Some(o);
            if spent(&self.runs[o]) {
                self.remove(o);
            }
            if spent(&self.runs[c]) {
                let next = self.runs[c].next;
                self.remove(c);
                closer = next;
            }
        }
        match floor {
            None => {
                self.bottom = None;
                self.top = None;
            }
            Some(f) => {
                self.runs[f].next = None;
                self.top = Some(f);
            }
        }
    }

    fn pairs(&mut self, opener: usize, closer: usize) -> bool {
        let (o, c) = (&self.runs[opener], &self.runs[closer]);
        if c.character == 0x7C {
            let (closer_start, opener_end) = (c.start, o.end);
            let visible = self.visible_counts();
            return visible[closer_start] > visible[opener_end];
        }
        let (o, c) = (&self.runs[opener], &self.runs[closer]);
        c.character == 0x7E
            || !((o.closes || c.opens)
                && (o.length + c.length) % 3 == 0
                && !(o.length % 3 == 0 && c.length % 3 == 0))
    }

    fn visible_counts(&mut self) -> &Vec<usize> {
        let source = self.source;
        self.visible.get_or_insert_with(|| {
            let mut counts = vec![0usize; source.len() + 1];
            for (index, &unit) in source.iter().enumerate() {
                counts[index + 1] = counts[index] + usize::from(!is_js_whitespace(unit));
            }
            counts
        })
    }

    fn remove(&mut self, run: usize) {
        let (previous, next) = (self.runs[run].previous, self.runs[run].next);
        match previous {
            None => self.bottom = next,
            Some(p) => self.runs[p].next = next,
        }
        match next {
            None => self.top = previous,
            Some(n) => self.runs[n].previous = previous,
        }
    }
}

fn spent(run: &Run) -> bool {
    run.count == 0 || (run.character == 0x7C && run.count < 2)
}

fn rune_at(source: &[u16], index: usize) -> char {
    scalar_at(source, index).map_or(REPLACEMENT, |(c, _)| c)
}

fn space(rune: Option<char>) -> bool {
    match rune {
        None => true,
        Some(c) => u16::try_from(u32::from(c)).is_ok_and(is_js_whitespace),
    }
}

fn punctuation(rune: Option<char>) -> bool {
    rune.is_some_and(|c| is_punctuation(c) || is_symbol(c))
}

/// The fenced blocks of a body, in order: at least three backticks and an info
/// string at the start of a line, with a line ending in a newline, up to the
/// first later line with the same backticks and nothing but spaces or tabs.
/// Older messages can use extra opening backticks and a triple closer. A fence
/// that never closes is prose, and it ends the search: no later fence could
/// close either.
pub fn fenced_ranges(text: &[u16]) -> Vec<(usize, usize)> {
    const FENCE: [u16; 3] = [BACKTICK; 3];
    let mut found = Vec::new();
    let mut closings = None;
    let mut line = 0;
    while line < text.len() {
        let info_end = line_end(text, line);
        if !text[line..].starts_with(&FENCE)
            || info_end >= text.len()
            || text[info_end] != b'\n' as u16
        {
            line = info_end + 1;
            continue;
        }

        let length = run_length(text, line, BACKTICK);
        let marker = if length > FENCE.len()
            && closings.get_or_insert_with(|| last_fence_closings(text))
                .get(&length).is_some_and(|&closing| closing > line)
        {
            &text[line..line + length]
        } else {
            &FENCE
        };
        let mut closing = info_end + 1;
        loop {
            let closing_end = line_end(text, closing);
            if closing <= text.len()
                && text[closing..closing_end].starts_with(marker)
                && is_blank_only(&text[closing + marker.len()..closing_end])
            {
                found.push((line, closing_end));
                line = closing_end + 1;
                break;
            }
            if closing_end >= text.len() {
                return found;
            }
            closing = closing_end + 1;
        }
    }
    found
}

/// Index longer closers once so falling back to triple fences stays linear.
fn last_fence_closings(text: &[u16]) -> HashMap<usize, usize> {
    let mut closings = HashMap::new();
    let mut line = 0;
    while line < text.len() {
        let end = line_end(text, line);
        let length = run_length(text, line, BACKTICK);
        if length >= 4 && is_blank_only(&text[line + length..end]) {
            closings.insert(length, line);
        }
        line = end + 1;
    }
    closings
}

pub fn line_end(text: &[u16], from: usize) -> usize {
    let mut end = from;
    while end < text.len() && !matches!(text[end], 0x0A | 0x0D | 0x2028 | 0x2029) {
        end += 1;
    }
    end
}

fn is_blank_only(text: &[u16]) -> bool {
    text.iter()
        .all(|&unit| unit == b' ' as u16 || unit == b'\t' as u16)
}

/// The runs of backticks, by length, with each length's cursor moving
/// forward once.
pub struct BacktickRuns {
    starts: HashMap<usize, Vec<usize>>,
    cursors: HashMap<usize, usize>,
}

impl BacktickRuns {
    pub fn new(text: &[u16]) -> BacktickRuns {
        let mut starts: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut index = 0;
        while index < text.len() {
            match text[index..].iter().position(|&unit| unit == BACKTICK) {
                None => break,
                Some(found) => index += found,
            }
            let run = run_length(text, index, BACKTICK);
            starts.entry(run).or_default().push(index);
            index += run;
        }
        BacktickRuns {
            starts,
            cursors: HashMap::new(),
        }
    }

    pub fn next(&mut self, length: usize, from: usize) -> Option<usize> {
        let list = self.starts.get(&length)?;
        let cursor = self.cursors.entry(length).or_insert(0);
        while *cursor < list.len() && list[*cursor] < from {
            *cursor += 1;
        }
        list.get(*cursor).copied()
    }
}

/// The parenthesis that balances each `(`, with the count restarting at
/// whitespace, which no address contains. Worked out once for the whole body
/// so a malformed link cannot make the scan quadratic.
struct ClosingParentheses {
    closing: Vec<Option<usize>>,
}

impl ClosingParentheses {
    fn new(text: &[u16]) -> ClosingParentheses {
        let mut closing = vec![None; text.len()];
        let mut opened: Vec<usize> = Vec::new();
        for (index, &current) in text.iter().enumerate() {
            if current == b'(' as u16 {
                opened.push(index);
            } else if current == b')' as u16 {
                if let Some(open) = opened.pop() {
                    closing[open] = Some(index);
                }
            } else if is_blank_unit(current) {
                opened.clear();
            }
        }
        ClosingParentheses { closing }
    }

    fn of(&self, open: usize) -> Option<usize> {
        self.closing.get(open).copied().flatten()
    }
}

fn starts_with_ascii(text: &[u16], prefix: &str, ignore_case: bool) -> bool {
    let prefix = prefix.as_bytes();
    text.len() >= prefix.len()
        && text.iter().zip(prefix).all(|(&unit, &byte)| {
            if ignore_case {
                unit < 128 && (unit as u8).eq_ignore_ascii_case(&byte)
            } else {
                unit == u16::from(byte)
            }
        })
}

/// `(http://...)` right after a `]`: a valid link target with a balanced
/// parenthesis, no whitespace, and something after the scheme. Returns where
/// the target ends.
fn link_target(text: &[u16], open: usize, parens: &ClosingParentheses) -> Option<usize> {
    if open >= text.len() || text[open] != b'(' as u16 {
        return None;
    }
    let scheme = &text[open + 1..];
    let scheme_length = if starts_with_ascii(scheme, "http://", true) {
        7
    } else if starts_with_ascii(scheme, "https://", true) {
        8
    } else {
        return None;
    };
    let close = parens.of(open)?;
    if close <= open + 1 + scheme_length {
        return None;
    }
    Some(close + 1)
}

/// Where a bare address starting at `index` ends, and where its match ends,
/// or `None` when there is no address there. Only lower-case schemes are
/// addresses here, exactly as in the transcript's linkify pass. The end is
/// trimmed the same way: sentence punctuation and a closing bracket that was
/// not opened inside the address are not part of it.
pub fn bare_address_end(text: &[u16], index: usize) -> Option<(usize, usize)> {
    let rest = &text[index..];
    let scheme_length = if starts_with_ascii(rest, "http://", false) {
        7
    } else if starts_with_ascii(rest, "https://", false) {
        8
    } else {
        return None;
    };
    if index + scheme_length >= text.len() || !is_address_character(text[index + scheme_length]) {
        return None;
    }
    let mut match_end = index + scheme_length;
    while match_end < text.len() && is_address_character(text[match_end]) {
        match_end += 1;
    }
    let end = trim_address(text, index, match_end, false);
    (end > index).then_some((end, match_end))
}

fn trim_address(text: &[u16], index: usize, match_end: usize, formatting: bool) -> usize {
    let (mut open_parens, mut close_parens, mut open_brackets, mut close_brackets) = (0, 0, 0, 0);
    for &unit in &text[index..match_end] {
        match unit {
            0x28 => open_parens += 1,
            0x29 => close_parens += 1,
            0x5B => open_brackets += 1,
            0x5D => close_brackets += 1,
            _ => {}
        }
    }

    let mut trimmed = match_end;
    while trimmed > index {
        let last = text[trimmed - 1];
        if is_trailing_punctuation(last)
            || (formatting && matches!(last, 0x2A | 0x5F | 0x7E | 0x7C))
        {
            trimmed -= 1;
            continue;
        }
        if last == 0x29 && close_parens > open_parens {
            close_parens -= 1;
            trimmed -= 1;
            continue;
        }
        if last == 0x5D && close_brackets > open_brackets {
            close_brackets -= 1;
            trimmed -= 1;
            continue;
        }
        break;
    }
    trimmed
}

/* ------------------------------------------------------------- characters */

pub fn run_length(text: &[u16], from: usize, character: u16) -> usize {
    let mut end = from;
    while end < text.len() && text[end] == character {
        end += 1;
    }
    end - from
}

fn exclude(excluded: &mut [bool], from: usize, to: usize) {
    let end = to.min(excluded.len());
    if from < end {
        excluded[from..end].fill(true);
    }
}

/// The characters a bare address may hold between its scheme and the next
/// space.
pub fn is_address_character(unit: u16) -> bool {
    !is_js_whitespace(unit) && !matches!(unit, 0x3C | 0x3E | 0x22 | 0x27 | 0x60)
}

fn is_trailing_punctuation(unit: u16) -> bool {
    matches!(
        unit,
        0x2E | 0x2C | 0x3B | 0x3A | 0x21 | 0x3F | 0x2026 | 0x27 | 0x22
    )
}

/// Whitespace or a control character, which no address and no valid link
/// target holds.
fn is_blank_unit(unit: u16) -> bool {
    unit <= 0x20 || unit == 0x7F || is_js_whitespace(unit)
}

/// How many UTF-16 units the name character at `index` takes, or zero.
fn name_character_width(content: &[u16], index: usize) -> usize {
    match scalar_at(content, index) {
        Some((c, width)) if is_name_character(c) => width,
        _ => 0,
    }
}

/// Whether the code point before `index` is a name character in visible text.
fn name_character_before(content: &[u16], index: usize, excluded: &[bool]) -> bool {
    let mut start = index - 1;
    if start > 0 && is_low_surrogate(content[start]) && is_high_surrogate(content[start - 1]) {
        start -= 1;
    }
    !excluded[start] && name_character_width(content, start) > 0
}

fn is_name_character(c: char) -> bool {
    is_letter_or_digit(c) || matches!(c, '_' | '.' | '-')
}

/// Whether `unit` can start a username character, for the rewriting's check.
pub fn starts_name(text: &[u16], index: usize) -> bool {
    match scalar_at(text, index) {
        Some((c, _)) => is_name_character(c),
        None => false,
    }
}
