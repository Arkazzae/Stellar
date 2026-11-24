//! The TZif file (RFC 8536): the zone's recorded transitions and, from
//! version 2 on, the rule that goes on after the last of them.

use crate::posix::{LocalType, Rule};

/// The longest a footer may be; real ones are under 100 bytes.
const MAX_FOOTER: usize = 256;
/// The largest offset the format allows: a little over 26 hours.
const MAX_OFFSET: i32 = 93_599;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tzif {
    /// Strictly ascending seconds since the epoch.
    transitions: Vec<i64>,
    /// For each transition, the index into `types` that applies from it on.
    indices: Vec<u8>,
    types: Vec<LocalType>,
    /// What applies before the first transition.
    before: usize,
    footer: Option<Rule>,
}

/// Why a file is not a usable TZif file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Malformed(pub &'static str);

struct Header {
    isut: usize,
    isstd: usize,
    leap: usize,
    time: usize,
    types: usize,
    chars: usize,
}

impl Header {
    /// Reads the 44 bytes at the front of `bytes`.
    fn read(bytes: &[u8]) -> Result<(u8, Header), Malformed> {
        if bytes.len() < 44 {
            return Err(Malformed("the header is cut short"));
        }
        if &bytes[..4] != b"TZif" {
            return Err(Malformed("the file does not start with TZif"));
        }
        let count = |at: usize| -> usize {
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize
        };
        Ok((
            bytes[4],
            Header {
                isut: count(20),
                isstd: count(24),
                leap: count(28),
                time: count(32),
                types: count(36),
                chars: count(40),
            },
        ))
    }

    /// The size of the data block that follows the header, with `size`-byte
    /// transition times.
    fn data_length(&self, size: usize) -> Option<usize> {
        let leap_entry = size.checked_add(4)?;
        self.time
            .checked_mul(size)?
            .checked_add(self.time)?
            .checked_add(self.types.checked_mul(6)?)?
            .checked_add(self.chars)?
            .checked_add(self.leap.checked_mul(leap_entry)?)?
            .checked_add(self.isstd)?
            .checked_add(self.isut)
    }
}

impl Tzif {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Tzif, Malformed> {
        let (version, first) = Header::read(bytes)?;
        match version {
            0 => {
                let body = block(&bytes[44..], &first, 4)?;
                Tzif::from_block(body, &first, 4, None)
            }
            b'2'..=b'9' => {
                let skipped = first
                    .data_length(4)
                    .ok_or(Malformed("a count is too large"))?;
                let rest = bytes[44..]
                    .get(skipped..)
                    .ok_or(Malformed("the first block is cut short"))?;
                let (_, second) = Header::read(rest)?;
                let after_header = &rest[44..];
                let body = block(after_header, &second, 8)?;
                let used = second.data_length(8).unwrap_or(usize::MAX);
                let footer = footer_of(&after_header[used..])?;
                Tzif::from_block(body, &second, 8, footer)
            }
            _ => Err(Malformed("the version is not known")),
        }
    }

    fn from_block(
        body: &[u8],
        header: &Header,
        size: usize,
        footer: Option<Rule>,
    ) -> Result<Tzif, Malformed> {
        if header.types == 0 {
            return Err(Malformed("there is no local time type"));
        }
        if header.chars == 0 {
            return Err(Malformed("there are no abbreviation characters"));
        }
        if (header.isstd != 0 && header.isstd != header.types)
            || (header.isut != 0 && header.isut != header.types)
        {
            return Err(Malformed("an indicator count does not match the types"));
        }
        let (times, rest) = body.split_at(header.time * size);
        let (indices, rest) = rest.split_at(header.time);
        let (infos, rest) = rest.split_at(header.types * 6);
        let chars = &rest[..header.chars];

        let mut transitions = Vec::with_capacity(header.time);
        for chunk in times.chunks_exact(size) {
            let moment = if size == 4 {
                i64::from(i32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            } else {
                let mut eight = [0u8; 8];
                eight.copy_from_slice(chunk);
                i64::from_be_bytes(eight)
            };
            if transitions.last().is_some_and(|last| *last >= moment) {
                return Err(Malformed("the transitions are not in ascending order"));
            }
            transitions.push(moment);
        }
        if indices.iter().any(|i| usize::from(*i) >= header.types) {
            return Err(Malformed("a transition names a type that does not exist"));
        }

        let mut types = Vec::with_capacity(header.types);
        for info in infos.as_chunks::<6>().0 {
            let offset = i32::from_be_bytes([info[0], info[1], info[2], info[3]]);
            if !(-MAX_OFFSET..=MAX_OFFSET).contains(&offset) {
                return Err(Malformed("an offset is out of range"));
            }
            let dst = match info[4] {
                0 => false,
                1 => true,
                _ => return Err(Malformed("a daylight saving flag is neither 0 nor 1")),
            };
            let start = usize::from(info[5]);
            let tail = chars
                .get(start..)
                .ok_or(Malformed("an abbreviation starts outside the characters"))?;
            let length = tail
                .iter()
                .position(|b| *b == 0)
                .ok_or(Malformed("an abbreviation is not terminated"))?;
            let abbreviation = String::from_utf8_lossy(&tail[..length]).into_owned();
            types.push(LocalType {
                offset,
                dst,
                abbreviation,
            });
        }

        // Before the first transition the first standard time type applies,
        // as other readers do; with none, the first type.
        let before = types.iter().position(|t| !t.dst).unwrap_or(0);
        Ok(Tzif {
            transitions,
            indices: indices.to_vec(),
            types,
            before,
            footer,
        })
    }

    /// The local time type in force at `moment`, seconds since the epoch.
    pub(crate) fn at(&self, moment: i64) -> &LocalType {
        match self.transitions.last() {
            None => {
                if let Some(rule) = &self.footer {
                    return rule.at(moment);
                }
            }
            Some(last) if moment >= *last => {
                if let Some(rule) = &self.footer {
                    return rule.at(moment);
                }
            }
            Some(_) => {}
        }
        let after = self.transitions.partition_point(|t| *t <= moment);
        match after.checked_sub(1) {
            Some(index) => &self.types[usize::from(self.indices[index])],
            None => &self.types[self.before],
        }
    }
}

/// The data block at the front of `bytes`, which must be all there.
fn block<'a>(bytes: &'a [u8], header: &Header, size: usize) -> Result<&'a [u8], Malformed> {
    let length = header
        .data_length(size)
        .ok_or(Malformed("a count is too large"))?;
    bytes
        .get(..length)
        .ok_or(Malformed("the data is cut short"))
}

/// `\nrule\n` after the second block; an empty rule means none.
fn footer_of(bytes: &[u8]) -> Result<Option<Rule>, Malformed> {
    if bytes.first() != Some(&b'\n') {
        return Err(Malformed("the footer is missing"));
    }
    let end = bytes[1..]
        .iter()
        .take(MAX_FOOTER + 1)
        .position(|b| *b == b'\n')
        .ok_or(Malformed("the footer is not closed"))?;
    let text = &bytes[1..1 + end];
    if text.is_empty() {
        return Ok(None);
    }
    let text = std::str::from_utf8(text).map_err(|_| Malformed("the footer is not text"))?;
    Rule::parse(text)
        .map(Some)
        .ok_or(Malformed("the footer is not a valid rule"))
}

#[cfg(test)]
pub(crate) mod build {
    //! A TZif writer for tests, so that a rule's own effect can be tested
    //! without a transition table that already covers the dates.

    pub struct Spec<'a> {
        pub transitions: &'a [(i64, u8)],
        /// (offset, dst, abbreviation)
        pub types: &'a [(i32, bool, &'a str)],
        pub footer: Option<&'a str>,
        pub version: u8,
    }

    fn block(spec: &Spec<'_>, size: usize) -> Vec<u8> {
        let mut chars = Vec::new();
        let mut starts = Vec::new();
        for (_, _, name) in spec.types {
            starts.push(chars.len() as u8);
            chars.extend_from_slice(name.as_bytes());
            chars.push(0);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"TZif");
        out.push(spec.version);
        out.extend_from_slice(&[0; 15]);
        for count in [
            0,
            0,
            0,
            spec.transitions.len(),
            spec.types.len(),
            chars.len(),
        ] {
            out.extend_from_slice(&(count as u32).to_be_bytes());
        }
        for (moment, _) in spec.transitions {
            if size == 4 {
                out.extend_from_slice(&(*moment as i32).to_be_bytes());
            } else {
                out.extend_from_slice(&moment.to_be_bytes());
            }
        }
        for (_, index) in spec.transitions {
            out.push(*index);
        }
        for ((offset, dst, _), start) in spec.types.iter().zip(&starts) {
            out.extend_from_slice(&offset.to_be_bytes());
            out.push(u8::from(*dst));
            out.push(*start);
        }
        out.extend_from_slice(&chars);
        out
    }

    pub fn file(spec: &Spec<'_>) -> Vec<u8> {
        let mut out = block(spec, 4);
        if spec.version != 0 {
            out.extend(block(spec, 8));
            out.push(b'\n');
            out.extend_from_slice(spec.footer.unwrap_or("").as_bytes());
            out.push(b'\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::build::{Spec, file};
    use super::*;

    const WARSAW: [(i32, bool, &str); 3] = [
        (3600, false, "CET"),
        (7200, true, "CEST"),
        (4000, false, "LMT"),
    ];

    fn spec<'a>(footer: Option<&'a str>, version: u8) -> Spec<'a> {
        Spec {
            transitions: &[(1_000_000_000, 1), (1_010_000_000, 0)],
            types: &WARSAW,
            footer,
            version,
        }
    }

    #[test]
    fn the_recorded_transitions_decide_until_the_last_one() {
        let zone = Tzif::parse(&file(&spec(Some("CET-1CEST,M3.5.0,M10.5.0/3"), b'2'))).unwrap();

        assert_eq!(zone.at(999_999_999).abbreviation, "CET");
        assert_eq!(zone.at(1_000_000_000).abbreviation, "CEST");
        assert_eq!(zone.at(1_009_999_999).abbreviation, "CEST");
        assert_eq!(zone.at(1_010_000_000).abbreviation, "CET");
    }

    #[test]
    fn the_footer_rule_decides_from_the_last_transition_on() {
        let zone = Tzif::parse(&file(&spec(Some("CET-1CEST,M3.5.0,M10.5.0/3"), b'3'))).unwrap();

        // 2030-07-01 is summer time under the rule, whatever the table says.
        assert_eq!(zone.at(1_909_000_000).abbreviation, "CEST");
        assert_eq!(zone.at(1_893_000_000).abbreviation, "CET");
    }

    #[test]
    fn before_the_first_transition_the_first_standard_type_applies() {
        let zone = Tzif::parse(&file(&Spec {
            transitions: &[(0, 0)],
            types: &[(7200, true, "CEST"), (3600, false, "CET")],
            footer: None,
            version: b'2',
        }))
        .unwrap();

        assert_eq!(zone.at(-1).abbreviation, "CET");
    }

    #[test]
    fn a_version_one_file_has_no_footer_and_keeps_its_last_type() {
        let zone = Tzif::parse(&file(&spec(None, 0))).unwrap();

        assert_eq!(zone.at(1_900_000_000).abbreviation, "CET");
    }

    #[test]
    fn a_zone_with_only_a_rule_follows_it() {
        let zone = Tzif::parse(&file(&Spec {
            transitions: &[],
            types: &[(3600, false, "CET")],
            footer: Some("CET-1CEST,M3.5.0,M10.5.0/3"),
            version: b'2',
        }))
        .unwrap();

        assert_eq!(zone.at(1_909_000_000).abbreviation, "CEST");
    }

    #[test]
    fn every_cut_of_a_file_is_refused_without_a_panic() {
        let whole = file(&spec(Some("CET-1CEST,M3.5.0,M10.5.0/3"), b'2'));
        for length in 0..whole.len() {
            assert!(Tzif::parse(&whole[..length]).is_err(), "cut at {length}");
        }
        assert!(Tzif::parse(&whole).is_ok());
    }

    #[test]
    fn every_single_byte_change_is_read_or_refused_without_a_panic() {
        let whole = file(&spec(Some("CET-1CEST,M3.5.0,M10.5.0/3"), b'2'));
        for at in 0..whole.len() {
            for value in [0u8, 1, 0x7f, 0x80, 0xff] {
                let mut changed = whole.clone();
                changed[at] = value;
                if let Ok(zone) = Tzif::parse(&changed) {
                    let _ = zone.at(0);
                    let _ = zone.at(i64::MAX);
                    let _ = zone.at(i64::MIN);
                }
            }
        }
    }

    #[test]
    fn huge_counts_are_refused_before_anything_is_allocated() {
        let mut bytes = file(&spec(None, 0));
        bytes[32..36].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Tzif::parse(&bytes).is_err());
        let mut bytes = file(&spec(Some("UTC0"), b'2'));
        bytes[36..40].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Tzif::parse(&bytes).is_err());
    }

    #[test]
    fn what_the_format_forbids_is_refused() {
        let ok = spec(Some("CET-1CEST,M3.5.0,M10.5.0/3"), b'2');
        let mut backwards = file(&Spec {
            transitions: &[(5, 0), (4, 1)],
            ..ok
        });
        assert!(Tzif::parse(&backwards).is_err());
        backwards.clear();

        let bad_index = file(&Spec {
            transitions: &[(5, 9)],
            types: &WARSAW,
            footer: None,
            version: b'2',
        });
        assert!(Tzif::parse(&bad_index).is_err());

        let bad_offset = file(&Spec {
            transitions: &[],
            types: &[(i32::MIN, false, "X")],
            footer: None,
            version: b'2',
        });
        assert!(Tzif::parse(&bad_offset).is_err());

        assert!(Tzif::parse(b"").is_err());
        assert!(
            Tzif::parse(b"not a zone file at all, but long enough to hold a header....").is_err()
        );
        let mut unknown_version = file(&spec(None, b'2'));
        unknown_version[4] = b'Z';
        assert!(Tzif::parse(&unknown_version).is_err());
    }

    #[test]
    fn a_footer_that_is_not_a_rule_is_refused() {
        assert!(Tzif::parse(&file(&spec(Some("nonsense"), b'2'))).is_err());
        let mut missing = file(&spec(None, b'2'));
        missing.truncate(missing.len() - 2);
        assert!(Tzif::parse(&missing).is_err());
    }
}
