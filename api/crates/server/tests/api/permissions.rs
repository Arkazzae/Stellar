//! The permission rules against the first server's own code, on the same
//! inputs.
//!
//! Not part of the ordinary run: it needs a .NET SDK and a small program, kept
//! outside the repository, that feeds the first server's published assembly
//! the inputs this test sends and answers on its standard output. The program
//! takes `evaluate` (27 little-endian words in, one word out, per case) or
//! `lines` (one command per line, one answer per line); the commands are
//! the ones the tests below send.
//!
//! ```sh
//! PERMISSIONS_HARNESS=/path/to/Harness.dll DOTNET=~/.dotnet/dotnet \
//!   cargo test -p stellarr-api --test api permissions:: -- --ignored --nocapture --skip arrange_servers
//! ```
//!
//! The pure parts are compared on every case of a space small enough to walk
//! (every override of every level, base role, held roles, owner, channel or
//! not, for one tracked permission at a time) and on some hundreds of
//! thousands of random ones, masks of the full 64 bits included.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;

use stellarr_api::permissions::evaluator::{self, Input, Level, Masks};
use stellarr_api::permissions::membership::parse_guid;
use stellarr_api::permissions::privileges::{self, ADMINISTRATOR, ALL, SEND_MESSAGES, VIEW_CHANNEL};
use stellarr_api::permissions::roles::{self, Role};
use stellarr_api::permissions::audit::{self, AuditChange};
use stellarr_api::values::Id;

fn harness(mode: &str) -> Child {
    let dll = std::env::var("PERMISSIONS_HARNESS").expect("PERMISSIONS_HARNESS: the harness assembly");
    let dotnet = std::env::var("DOTNET").unwrap_or_else(|_| "dotnet".to_string());
    Command::new(dotnet)
        .arg(dll)
        .arg(mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the harness starts")
}

/// A small, fast generator that is the same on every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }

    /// A mask of some shape a stored one could have: nothing, one bit, a few,
    /// everything, an unknown bit, the sign bit, or anything at all.
    fn mask(&mut self) -> i64 {
        match self.below(10) {
            0 => 0,
            1 | 2 => 1 << self.below(15),
            3 | 4 => (self.next() as i64) & ALL,
            5 => ALL,
            6 => (self.next() as i64) & ALL | (1 << (15 + self.below(48))),
            7 => i64::MIN | (self.next() as i64 & ALL),
            8 => -1,
            _ => self.next() as i64,
        }
    }
}

/// One case as the harness reads it: 27 little-endian words.
fn record(
    owner: bool,
    at_channel: bool,
    base: i64,
    held: &[i64],
    levels: &[Level],
    out: &mut Vec<u8>,
) {
    let mut words = [0_i64; 27];
    words[0] = i64::from(owner) | (i64::from(at_channel) << 1);
    words[1] = held.len() as i64;
    words[2] = levels.len() as i64;
    words[3] = base;
    for (i, h) in held.iter().enumerate() {
        words[4 + i] = *h;
    }
    for (l, level) in levels.iter().enumerate() {
        let o = 7 + l * 10;
        words[o] = level.base.allow;
        words[o + 1] = level.base.deny;
        for (i, role) in level.held.iter().enumerate() {
            words[o + 2 + i * 2] = role.allow;
            words[o + 3 + i * 2] = role.deny;
        }
        words[o + 8] = level.member.allow;
        words[o + 9] = level.member.deny;
    }
    for word in words {
        out.extend_from_slice(&word.to_le_bytes());
    }
}

/// Sends every case the generator makes to the harness and compares each
/// answer with this implementation's. Returns the number of cases and of
/// differences.
fn compare_evaluations(generate: impl FnOnce(&mut dyn FnMut(bool, bool, i64, &[i64], &[Level])) + Send + 'static) -> (u64, u64) {
    let mut child = harness("evaluate");
    let mut stdin: ChildStdin = child.stdin.take().expect("stdin");
    let mut stdout: ChildStdout = child.stdout.take().expect("stdout");
    let (sender, receiver) = mpsc::sync_channel::<Vec<i64>>(8);
    let writer = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut expected = Vec::new();
        let mut push = |owner: bool, at_channel: bool, base: i64, held: &[i64], levels: &[Level]| {
            record(owner, at_channel, base, held, levels, &mut bytes);
            expected.push(evaluator::evaluate(&Input { is_owner: owner, base, held, levels, at_channel }));
            if expected.len() == 20_000 {
                // Tell the reader first: the harness answers as it reads, and
                // stops reading when nobody takes its answers.
                sender.send(std::mem::take(&mut expected)).expect("send");
                stdin.write_all(&bytes).expect("write");
                bytes.clear();
            }
        };
        generate(&mut push);
        if !expected.is_empty() {
            sender.send(expected).expect("send");
            stdin.write_all(&bytes).expect("write");
        }
        // Closing the pipe ends the harness.
    });
    let (mut cases, mut differences) = (0_u64, 0_u64);
    for expected in receiver {
        let mut answers = vec![0u8; expected.len() * 8];
        stdout.read_exact(&mut answers).expect("answers");
        for (mine, theirs) in expected.iter().zip(answers.chunks_exact(8)) {
            cases += 1;
            let theirs = i64::from_le_bytes(theirs.try_into().expect("8 bytes"));
            if *mine != theirs {
                differences += 1;
                if differences < 10 {
                    eprintln!("DIFFERENT: ours {mine:#x}, theirs {theirs:#x}");
                }
            }
        }
    }
    writer.join().expect("writer");
    child.wait().expect("harness exits");
    (cases, differences)
}

const STATES: [fn(i64) -> Masks; 4] = [
    |_| Masks::new(0, 0),
    |bit| Masks::new(bit, 0),
    |bit| Masks::new(0, bit),
    |bit| Masks::new(bit, bit),
];

fn level_of(slots: &[usize], bit: i64, held: usize) -> Level {
    Level {
        base: STATES[slots[0]](bit),
        held: (0..held).map(|i| STATES[slots[1 + i]](bit)).collect(),
        member: STATES[slots[3]](bit),
    }
}

#[test]
#[ignore = "needs the first server's code: see the top of this file"]
fn the_evaluator_gives_the_first_servers_answer_for_every_override_of_one_permission() {
    let bits = [
        VIEW_CHANNEL,
        ADMINISTRATOR,
        SEND_MESSAGES,
        privileges::MANAGE_MESSAGES,
        1 << 20,
        i64::MIN,
    ];
    let (cases, differences) = compare_evaluations(move |push| {
        for bit in bits {
            for owner in [false, true] {
                for at_channel in [false, true] {
                    for base in [0, bit] {
                        for held_bits in 0..4_i64 {
                            let held = [if held_bits & 1 != 0 { bit } else { 0 }, if held_bits & 2 != 0 { bit } else { 0 }];
                            // No level, one level, two levels, each slot in each of four states.
                            push(owner, at_channel, base, &held, &[]);
                            for code in 0..4_usize.pow(4) {
                                let slots = [code % 4, code / 4 % 4, code / 16 % 4, code / 64 % 4];
                                let one = level_of(&slots, bit, 2);
                                push(owner, at_channel, base, &held, std::slice::from_ref(&one));
                            }
                            for code in 0..4_usize.pow(8) {
                                let first = [code % 4, code / 4 % 4, code / 16 % 4, code / 64 % 4];
                                let second = [code / 256 % 4, code / 1024 % 4, code / 4096 % 4, code / 16384 % 4];
                                let levels = [level_of(&first, bit, 2), level_of(&second, bit, 2)];
                                push(owner, at_channel, base, &held, &levels);
                            }
                        }
                    }
                }
            }
        }
    });
    println!("every override of one permission: {cases} cases, {differences} differences");
    assert_eq!(differences, 0);
}

#[test]
#[ignore = "needs the first server's code: see the top of this file"]
fn the_evaluator_gives_the_first_servers_answer_where_view_and_another_permission_meet() {
    // Two permissions at once, one level: each override says nothing, allows,
    // denies or both for each of the two, for the base role, two held roles
    // and the member; every role holds either, both or none.
    let view = VIEW_CHANNEL;
    let send = SEND_MESSAGES;
    let masks = move |code: usize| {
        let part = |state: usize, bit: i64| STATES[state](bit);
        let (a, b) = (part(code % 4, view), part(code / 4 % 4, send));
        Masks::new(a.allow | b.allow, a.deny | b.deny)
    };
    let (cases, differences) = compare_evaluations(move |push| {
        for owner in [false, true] {
            for at_channel in [false, true] {
                for perms in 0..4_i64.pow(3) {
                    let pick = |n: i64| [0, view, send, view | send][(n % 4) as usize];
                    let (base, h1, h2) = (pick(perms), pick(perms / 4), pick(perms / 16));
                    for code in 0..16_usize.pow(4) {
                        let level = Level {
                            base: masks(code % 16),
                            held: vec![masks(code / 16 % 16), masks(code / 256 % 16)],
                            member: masks(code / 4096 % 16),
                        };
                        push(owner, at_channel, base, &[h1, h2], std::slice::from_ref(&level));
                    }
                }
            }
        }
    });
    println!("view with another permission: {cases} cases, {differences} differences");
    assert_eq!(differences, 0);
}

#[test]
#[ignore = "needs the first server's code: see the top of this file"]
fn the_evaluator_gives_the_first_servers_answer_on_random_cases_with_full_masks() {
    let count: u64 = std::env::var("PERMISSIONS_RANDOM_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(600_000);
    let (cases, differences) = compare_evaluations(move |push| {
        let mut random = Random(0x9e37_79b9_7f4a_7c15);
        for _ in 0..count {
            let held_count = random.below(4) as usize;
            let level_count = random.below(3) as usize;
            let held: Vec<i64> = (0..held_count).map(|_| random.mask()).collect();
            let levels: Vec<Level> = (0..level_count)
                .map(|_| Level {
                    base: Masks::new(random.mask(), random.mask()),
                    held: (0..held_count).map(|_| Masks::new(random.mask(), random.mask())).collect(),
                    member: Masks::new(random.mask(), random.mask()),
                })
                .collect();
            let (owner, at_channel) = (random.chance(8), random.chance(2));
            let base = random.mask();
            push(owner, at_channel, base, &held, &levels);
        }
    });
    println!("random cases: {cases} cases, {differences} differences");
    assert_eq!(differences, 0);
}

fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(DIGITS[(n >> (18 - 6 * i) & 63) as usize]));
        }
        for _ in chunk.len()..3 {
            out.push('=');
        }
    }
    out
}

fn unbase64(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let mut out = Vec::new();
    let digits: Vec<u8> = text.bytes().filter(|c| *c != b'=').map(value).collect();
    for chunk in digits.chunks(4) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, d)| n | u32::from(*d) << (18 - 6 * i));
        for i in 0..chunk.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    out
}

fn text_arg(text: Option<&str>) -> String {
    text.map_or_else(|| "-".to_string(), |t| format!("~{}", base64(t.as_bytes())))
}

fn text_result(line: &str) -> Option<String> {
    (line != "-").then(|| String::from_utf8(unbase64(&line[1..])).expect("utf-8"))
}

struct Lines {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Lines {
    fn start() -> Lines {
        let mut child = harness("lines");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Lines { child, stdin, stdout }
    }

    fn ask(&mut self, line: &str) -> String {
        writeln!(self.stdin, "{line}").expect("write");
        let mut answer = String::new();
        self.stdout.read_line(&mut answer).expect("read");
        answer.trim_end().to_string()
    }

    fn finish(mut self) {
        drop(self.stdin);
        self.child.wait().expect("exit");
    }
}

fn role_spec(role: &Role) -> String {
    format!(
        "{},{},{},{},{},{}",
        role.server,
        role.id,
        role.position,
        u8::from(role.is_owner),
        u8::from(role.is_system),
        role.permissions
    )
}

fn random_id(random: &mut Random, pool: u64) -> Id {
    // Few distinct leading bytes, so that ordering has to look past them.
    let mut bytes = [0u8; 16];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = if i < 12 { (random.below(pool)) as u8 } else { random.next() as u8 };
    }
    Id::from_bytes(bytes)
}

fn random_role(random: &mut Random, server: Id) -> Role {
    Role {
        id: random_id(random, 3),
        server,
        position: random.below(4) as i64,
        is_system: random.chance(3),
        is_owner: random.chance(5),
        permissions: random.mask(),
    }
}

#[test]
#[ignore = "needs the first server's code: see the top of this file"]
fn ranking_rank_and_the_small_pure_checks_give_the_first_servers_answers() {
    let mut lines = Lines::start();
    let mut random = Random(0x1234_5678_9abc_def1);
    let mut differences = 0_u64;
    let mut cases = 0_u64;
    let mut check = |what: &str, ours: String, theirs: String| {
        cases += 1;
        if ours != theirs {
            differences += 1;
            if differences < 10 {
                eprintln!("DIFFERENT {what}: ours {ours:?}, theirs {theirs:?}");
            }
        }
    };
    let servers = [Id::from_bytes([1; 16]), Id::from_bytes([2; 16])];
    for _ in 0..20_000 {
        let server = servers[random.below(2) as usize];
        let held: Vec<Role> = (0..random.below(6)).map(|_| random_role(&mut random, server)).collect();
        let specs = held.iter().map(role_spec).collect::<Vec<_>>().join(";");
        let ids = |roles: &[Role]| roles.iter().map(|r| r.id.to_string()).collect::<Vec<_>>().join(",");
        check("ranked", ids(&roles::ranked(held.clone())), lines.ask(&format!("ranked {specs}")));

        let base = random_role(&mut random, server);
        check(
            "top",
            roles::top(&held, base).id.to_string(),
            lines.ask(&format!("top {specs} {}", role_spec(&base))),
        );

        let actor_server = servers[random.below(2) as usize];
        let actor = random_role(&mut random, actor_server);
        let target_server = servers[random.below(2) as usize];
        let target = random_role(&mut random, target_server);
        check(
            "outranks",
            u8::from(actor.outranks(&target)).to_string(),
            lines.ask(&format!("outranks {} {}", role_spec(&actor), role_spec(&target))),
        );

        // `of`: a member of `server` whose stored roles are some of the server's and some strangers'.
        let mut pool: Vec<Role> = Vec::new();
        for _ in 0..random.below(6) {
            let pool_server = servers[random.below(2) as usize];
            pool.push(random_role(&mut random, pool_server));
        }
        let mut unique = std::collections::HashMap::new();
        for r in &pool {
            unique.entry(r.id).or_insert(*r);
        }
        let pool: Vec<Role> = unique.values().copied().collect();
        let base_id = (!pool.is_empty() && random.chance(2)).then(|| pool[random.below(pool.len() as u64) as usize].id);
        let top_role = random_id(&mut random, 3);
        let stored: Vec<Id> = (0..random.below(4))
            .map(|_| if !pool.is_empty() && random.chance(4) { pool[random.below(pool.len() as u64) as usize].id } else { random_id(&mut random, 3) })
            .collect();
        let by_id: std::collections::HashMap<Id, Role> = pool.iter().map(|r| (r.id, *r)).collect();
        let theirs = lines.ask(&format!(
            "of {server} {top_role} {} {} {}",
            stored.iter().map(Id::to_string).collect::<Vec<_>>().join(","),
            if pool.is_empty() { String::new() } else { pool.iter().map(role_spec).collect::<Vec<_>>().join(";") },
            base_id.map_or("-".to_string(), |b| b.to_string()),
        ));
        // A pool that is empty cannot name a base role; the harness reads an empty list as none.
        let ours = ids(&roles::held_by(server, top_role, &stored, &by_id, base_id));
        check("held roles", ours, theirs);
    }
    for _ in 0..40_000 {
        let n: Vec<i64> = (0..6).map(|_| random.mask()).collect();
        let masks = |i: usize| Masks::new(n[i], n[i + 1]);
        check(
            "changed",
            privileges_changed(masks(0), masks(2), masks(4)).to_string(),
            lines.ask(&format!("changed {} {} {} {} {} {}", n[0], n[1], n[2], n[3], n[4], n[5])),
        );
        let (a, b) = (random.mask(), random.mask());
        check("valid", u8::from(privileges::is_valid(a)).to_string(), lines.ask(&format!("valid {a}")));
        check(
            "overridable",
            u8::from(privileges::is_overridable(a, b)).to_string(),
            lines.ask(&format!("overridable {a} {b}")),
        );
        check(
            "contains",
            u8::from(privileges::contains(a, b)).to_string(),
            lines.ask(&format!("contains {a} {b}")),
        );
        check("names", privileges::to_names(a).join(","), lines.ask(&format!("names {a}")));
    }
    let known = ["manageServer", "manageBans", "viewChannel", "administrator", "ManageServer", "", "superuser", "connect"];
    for _ in 0..5_000 {
        let names: Vec<&str> = (0..random.below(5)).map(|_| known[random.below(known.len() as u64) as usize]).collect();
        let ours = privileges::parse_names(names.iter().copied())
            .map_or("0 0".to_string(), |mask| format!("1 {mask}"));
        let args = names.iter().map(|n| text_arg(Some(n))).collect::<Vec<_>>().join(" ");
        check("parse names", ours, lines.ask(&format!("parse {args}")));
    }
    println!("ranking and the small checks: {cases} cases, {differences} differences");
    lines.finish();
    assert_eq!(differences, 0);
}

fn privileges_changed(a: Masks, b: Masks, c: Masks) -> i64 {
    stellarr_api::permissions::authorization::changed(a, b, c)
}

const ALPHABET: &[&str] = &[
    "a", "Z", " ", "ó", "ł", "ż", "Ś", "\"", "\\", "/", "'", "<", ">", "&", "+", "`", "=", "\n", "\t", "\r", "\u{8}", "\u{c}",
    "\u{0}", "\u{1f}", "\u{7f}", "\u{80}", "é", "😀", "🇵🇱", "\u{2026}", "\u{200b}", "\u{ffff}", "-", "_", "~", ":",
];

fn random_text(random: &mut Random, longest: u64) -> String {
    (0..random.below(longest)).map(|_| ALPHABET[random.below(ALPHABET.len() as u64) as usize]).collect()
}

#[test]
#[ignore = "needs the first server's code: see the top of this file"]
fn text_clipped_and_written_for_the_audit_log_is_the_first_servers_text() {
    let mut lines = Lines::start();
    let mut random = Random(0xdead_beef_cafe_f00d);
    let (mut cases, mut differences) = (0_u64, 0_u64);
    // Every character of the ASCII range and of the first planes, alone.
    let mut singles: Vec<String> = (0..0x250_u32).filter_map(char::from_u32).map(String::from).collect();
    singles.extend((0x2000..0x2100_u32).filter_map(char::from_u32).map(String::from));
    singles.extend(["😀".to_string(), "\u{10ffff}".to_string(), "\u{e000}".to_string()]);
    for text in &singles {
        let change = AuditChange { field: text.clone(), from: Some(text.clone()), ..AuditChange::default() };
        let ours = audit::changes_json(std::slice::from_ref(&change));
        let theirs = lines.ask(&format!(
            "changes {} {} - - - - -",
            text_arg(Some(text)),
            text_arg(Some(text))
        ));
        cases += 1;
        if text_result(&theirs).as_deref() != Some(ours.as_str()) {
            differences += 1;
            if differences < 10 {
                eprintln!("DIFFERENT json of {text:?}: ours {ours}, theirs {:?}", text_result(&theirs));
            }
        }
    }
    for _ in 0..30_000 {
        let text = random_text(&mut random, 30);
        let max = 1 + random.below(12) as usize;
        let ours = audit::clip(&text, max);
        let theirs = text_result(&lines.ask(&format!("clip {} {max}", text_arg(Some(&text))))).expect("a text");
        cases += 1;
        if ours != theirs {
            differences += 1;
            if differences < 10 {
                eprintln!("DIFFERENT clip of {text:?} to {max}: ours {ours:?}, theirs {theirs:?}");
            }
        }

        let list = |random: &mut Random| -> Option<Vec<String>> {
            random.chance(2).then(|| (0..random.below(3)).map(|_| random_text(random, 6)).collect())
        };
        let optional = |random: &mut Random| random.chance(2).then(|| random_text(random, 8));
        let change = AuditChange {
            field: random_text(&mut random, 8),
            from: optional(&mut random),
            to: optional(&mut random),
            added: list(&mut random),
            removed: list(&mut random),
            from_color: optional(&mut random),
            to_color: optional(&mut random),
        };
        // The list goes to the harness as JSON text, escaped only as far as its own reader needs.
        let list_arg = |items: &Option<Vec<String>>| {
            text_arg(
                items
                    .as_ref()
                    .map(|items| serde_json::to_string(items).expect("a list is JSON"))
                    .as_deref(),
            )
        };
        let ours = audit::changes_json(std::slice::from_ref(&change));
        let theirs = text_result(&lines.ask(&format!(
            "changes {} {} {} {} {} {} {}",
            text_arg(Some(&change.field)),
            text_arg(change.from.as_deref()),
            text_arg(change.to.as_deref()),
            list_arg(&change.added),
            list_arg(&change.removed),
            text_arg(change.from_color.as_deref()),
            text_arg(change.to_color.as_deref()),
        )))
        .expect("a text");
        cases += 1;
        if ours != theirs {
            differences += 1;
            if differences < 10 {
                eprintln!("DIFFERENT changes {change:?}: ours {ours}, theirs {theirs}");
            }
        }
    }
    // What was stored, read back: lines as the servers write them, and lines that are damaged.
    // Not compared: a null where text is required (a null item, a null or missing `field`),
    // which no writer makes. The first server carries it on as a null; here an item that is
    // null makes the whole line unreadable and a field that is null or missing is empty.
    for stored in [
        r#"[{"field":"name","from":"a","to":"b"}]"#,
        r#"[{"Field":"name","FROM":"a"},{"field":"x","added":["p","q"],"removed":[],"fromColor":"red","toColor":null}]"#,
        r#"[]"#,
        r#"null"#,
        r#"{}"#,
        r#"[1]"#,
        r#"[{"field":1}]"#,
        r#"[{"field":"a","added":"p"}]"#,
        r#"[{"field":"a","added":["p",1]}]"#,
        r#"[{"field":"a","extra":5}]"#,
        r#"[{"field":"a"},"b"]"#,
        r#"[{"field":"a","to":{"x":1}}]"#,
        r#"[{"field":"a"#,
        r#"nonsense"#,
        r#"[{"field":"a","from":"\u00e9\ud83d\ude00\n\"\/"}]"#,
        r#"[{"field":"a","from":"\ud83d"}]"#,
        r#"[{"field":"a","from":"é\u00e9\ud83d\ude00"}]"#,
        r#"[{"field":"a","field":"b"}]"#,
        r#"[{"field":"a"}] trailing"#,
    ] {
        let ours = audit::changes_json(&audit::read_changes(Some(stored)));
        let theirs = text_result(&lines.ask(&format!("readchanges {}", text_arg(Some(stored))))).expect("a text");
        cases += 1;
        if ours != theirs {
            differences += 1;
            eprintln!("DIFFERENT read of {stored}: ours {ours}, theirs {theirs}");
        }
    }
    println!("audit text: {cases} cases, {differences} differences");
    lines.finish();
    assert_eq!(differences, 0);
}

#[test]
#[ignore = "needs the first server's code: see the top of this file"]
fn an_id_in_a_header_is_read_as_the_first_server_reads_one() {
    let mut lines = Lines::start();
    let mut random = Random(0x0bad_cafe_1234_5678);
    let (mut cases, mut differences) = (0_u64, 0_u64);
    let pieces = [
        "{", "}", "(", ")", "-", "0x", ",", " ", "\t", "\u{a0}", "\u{2003}", "\u{200b}", "0", "1", "a", "F", "g", "Z", "00000000", "0000", "000000000000", "00000000000000000000000000000000",
        "00000000-0000-0000-0000-000000000000", "{0x1,0x1,0x1,{0x1,0x1,0x1,0x1,0x1,0x1,0x1,0x1}}", "ab", "cdef",
    ];
    for round in 0..60_000 {
        let text: String = if round % 3 == 1 {
            // Groups spelled with a `0x` in front, and fewer digits, and other damage.
            let id = Id::random().to_string();
            let groups: Vec<String> = id
                .split('-')
                .map(|group| match random.below(6) {
                    0 => format!("0x{}", &group[2..]),
                    1 => format!("0X{}", &group[2..]),
                    2 => format!("0x{}", &group[2..]).replacen('0', "g", 1),
                    3 => format!("{}0x", &group[..group.len() - 2]),
                    4 => format!("0x{}", "0".repeat(group.len() - 2)),
                    _ => group.to_string(),
                })
                .collect();
            let joined = groups.join("-");
            match random.below(3) {
                0 => joined,
                1 => format!("{{{joined}}}"),
                _ => joined.replace('-', ""),
            }
        } else if round % 3 == 0 {
            // A real id in one of its spellings, then damaged.
            let id = Id::random().to_string();
            let spelled = match random.below(6) {
                0 => id.clone(),
                1 => id.replace('-', ""),
                2 => format!("{{{id}}}"),
                3 => format!("({id})"),
                4 => id.to_uppercase(),
                _ => format!(" {id}\t"),
            };
            let mut chars: Vec<char> = spelled.chars().collect();
            if random.chance(2) && !chars.is_empty() {
                let at = random.below(chars.len() as u64) as usize;
                match random.below(3) {
                    0 => {
                        chars.remove(at);
                    }
                    1 => chars.insert(at, ['-', 'g', ' ', '{', '0'][random.below(5) as usize]),
                    _ => chars[at] = ['-', 'g', ' ', '}', 'x'][random.below(5) as usize],
                }
            }
            chars.into_iter().collect()
        } else {
            (0..1 + random.below(8)).map(|_| pieces[random.below(pieces.len() as u64) as usize]).collect()
        };
        let ours = parse_guid(&text).map_or("0".to_string(), |id| format!("1 {id}"));
        let theirs = lines.ask(&format!("guid {}", text_arg(Some(&text))));
        cases += 1;
        if ours != theirs {
            differences += 1;
            if differences < 20 {
                eprintln!("DIFFERENT id {text:?}: ours {ours}, theirs {theirs}");
            }
        }
    }
    println!("ids: {cases} cases, {differences} differences");
    lines.finish();
    assert_eq!(differences, 0);
}

// ---- The same database in both servers

/// Arranges servers of random shape in a copy of the first server's database
/// and writes what these rules say each person sees, for
/// `tests/permissions-shared-database.sh` to compare with what the first
/// server answers on the same file.
///
/// `PERMISSIONS_DB` is the database file, `PERMISSIONS_USERS` a file of
/// `name id` lines of accounts that exist in it, `PERMISSIONS_EXPECTED` where
/// the answers are written, `PERMISSIONS_SERVERS` how many servers to make and
/// `PERMISSIONS_SEED` which shapes.
#[test]
#[ignore = "run by tests/permissions-shared-database.sh"]
fn arrange_servers_in_a_shared_database_and_write_what_each_person_sees() {
    use rusqlite::{Connection, params};
    use stellarr_api::permissions::{authorization, ownership, visibility};
    use stellarr_api::values::Timestamp;

    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
    let db = Connection::open(var("PERMISSIONS_DB")).expect("the database");
    db.pragma_update(None, "foreign_keys", true).expect("foreign keys");
    let users: Vec<(String, Id)> = std::fs::read_to_string(var("PERMISSIONS_USERS"))
        .expect("the users")
        .lines()
        .filter_map(|line| {
            let (name, id) = line.split_once(' ')?;
            Some((name.to_string(), Id::parse(id)?))
        })
        .collect();
    let servers: u64 = var("PERMISSIONS_SERVERS").parse().expect("a count");
    let mut random = Random(var("PERMISSIONS_SEED").parse::<u64>().expect("a seed") | 1);
    let mut out = String::new();

    // Permissions worth telling apart: the ones the rules treat in their own way, and a few others.
    let kinds = [
        privileges::VIEW_CHANNEL,
        privileges::SEND_MESSAGES,
        privileges::MANAGE_CHANNELS,
        privileges::MANAGE_MESSAGES,
        privileges::MANAGE_ROLES,
    ];
    let mask = |random: &mut Random| -> i64 {
        let ordinary = kinds.iter().filter(|_| random.chance(3)).fold(0, |all, bit| all | bit);
        // Administrator is rare, and an override that says it is ignored.
        if random.chance(14) { ordinary | privileges::ADMINISTRATOR } else { ordinary }
    };
    for number in 0..servers {
        let server = Id::random();
        db.execute(
            "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, ?2, '', 0, ?3, 0, 0)",
            params![server, format!("Shape {number}"), Timestamp::now()],
        )
        .expect("a server");
        let role = |name: &str, position: i64, permissions: i64, system: bool, owner: bool| -> Id {
            let id = Id::random();
            db.execute(
                "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, ?3, '#777777', ?4, ?5, ?6, ?7)",
                params![id, server, name, permissions, position, system, owner],
            )
            .expect("a role");
            id
        };
        let owner_role = role("Owner", 0, privileges::ALL, true, true);
        // Most servers keep the usual base role; some lose view or have none.
        let base_permissions = match random.below(5) {
            0 => privileges::SEND_MESSAGES,
            1 => 0,
            _ => privileges::BASE_ROLE_DEFAULTS,
        };
        let others: Vec<Id> = (0..1 + random.below(4))
            .map(|n| role(&format!("Role {n}"), 1 + random.below(3) as i64, mask(&mut random), false, false))
            .collect();
        let base_role = role("Member", 5, base_permissions, true, false);
        let base = ServerRolesBase::of(&db, server, base_role);
        let all_roles: Vec<Id> = std::iter::once(owner_role).chain(others.iter().copied()).chain([base_role]).collect();

        let sections: Vec<Id> = (0..random.below(4)).map(|_| Id::random()).collect();
        for (n, section) in sections.iter().enumerate() {
            db.execute(
                "INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, ?3, ?4)",
                params![section, server, format!("Section {n}"), n as i64],
            )
            .expect("a section");
        }
        let mut channels: Vec<(Id, Option<Id>)> = Vec::new();
        let mut texts: Vec<Id> = Vec::new();
        for n in 0..2 + random.below(8) {
            let id = Id::random();
            let section = (!sections.is_empty() && random.chance(2)).then(|| sections[random.below(sections.len() as u64) as usize]);
            let voice = random.chance(4);
            if !voice {
                texts.push(id);
            }
            db.execute(
                "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, ?3, ?4, ?5, '', 0, 0, 0, ?6)",
                params![id, server, format!("channel-{n}"), i64::from(voice), n as i64, section],
            )
            .expect("a channel");
            channels.push((id, section));
        }
        // People: the first is the owner, the rest hold some of the roles.
        for (position, (_, user)) in users.iter().enumerate() {
            let held: Vec<Id> = if position == 0 {
                vec![owner_role]
            } else {
                others.iter().copied().filter(|_| random.chance(3)).collect()
            };
            let by_id = stellarr_api::permissions::roles::ServerRoles::load(&db, server).expect("roles").by_id();
            let held_roles: Vec<Role> = held.iter().map(|id| by_id[id]).collect();
            let top = roles::top(&held_roles, base.role);
            ownership::admit(&db, server, *user, top.id, top.id == base.role.id, (position == 0).then_some(1), Timestamp::now()).expect("a member");
            ownership::set_roles(&db, server, *user, &held_roles, &base.role).expect("held roles");
        }
        // Overrides: of the roles and of members, at sections and channels, of every kind.
        for (section, place) in &places_for_overrides(&sections, &channels) {
            for role in &all_roles {
                if *role != owner_role && random.chance(3) {
                    let allow = mask(&mut random);
                    let deny = if random.chance(6) { allow & mask(&mut random) | mask(&mut random) } else { mask(&mut random) & !allow };
                    db.execute(
                        "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![Id::random(), server, i64::from(*section), place, role, allow, deny],
                    )
                    .expect("an override");
                }
            }
            for (_, user) in users.iter().skip(1) {
                if random.chance(6) {
                    db.execute(
                        "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![Id::random(), server, i64::from(*section), place, user, mask(&mut random), mask(&mut random)],
                    )
                    .expect("an override of a member");
                }
            }
        }
        // What these rules say each person sees on this server.
        let places: Vec<(bool, Id)> = sections.iter().map(|s| (true, *s)).chain(channels.iter().map(|(c, _)| (false, *c))).collect();
        for (name, user) in &users {
            let permissions = authorization::server_wide(&db, *user, server).expect("permissions");
            let view = visibility::of_member(&db, *user, server).expect("a view");
            let mut seen_channels: Vec<String> = Vec::new();
            let mut seen_sections: Vec<String> = Vec::new();
            let mut sending = serde_json::Map::new();
            for (channel, _) in &channels {
                let held = authorization::held_at(&db, *user, &authorization::Scope::channel(server, *channel)).expect("held");
                let visible = view.as_ref().is_none_or(|view| view.channels.contains(channel));
                assert_eq!(held.views(), visible, "{name}: the view of a channel is what the layout says");
                if visible {
                    seen_channels.push(channel.to_string());
                }
                if !texts.contains(channel) {
                    continue;
                }
                let answer = if !held.views() {
                    404
                } else if !held.has(privileges::SEND_MESSAGES) {
                    403
                } else {
                    201
                };
                sending.insert(channel.to_string(), answer.into());
            }
            for section in &sections {
                if view.as_ref().is_none_or(|view| view.sections.contains(section)) {
                    seen_sections.push(section.to_string());
                }
            }
            seen_channels.sort();
            seen_sections.sort();
            // What the routes that change roles and overrides would answer, by the order of
            // the checks they make: the place is there, the person may manage roles, the
            // request is well formed, and then the decision of the rule itself.
            let manages_roles = privileges::contains(permissions, privileges::MANAGE_ROLES);
            let mut probes = Vec::new();
            let wanted = if random.chance(2) { permissions & mask(&mut random) } else { mask(&mut random) };
            probes.push(serde_json::json!({
                "kind": "role", "permissions": privileges::to_names(wanted),
                "status": if manages_roles && authorization::can_create_role(&db, *user, server, wanted).expect("a check") { 201 } else { 403 },
            }));
            let masks = |random: &mut Random| -> (i64, i64) {
                let basis = permissions & ALL;
                let pick = |random: &mut Random| if random.chance(2) { basis & mask(random) } else { mask(random) };
                let allow = pick(random);
                let deny = if random.chance(10) { pick(random) } else { pick(random) & !allow };
                (allow, deny)
            };
            let existing = |table: &str, owner: &str, section: bool, place: Id, who: Id| -> bool {
                let count: i64 = db
                    .query_row(
                        &format!("SELECT COUNT(*) FROM \"{table}\" WHERE \"TargetType\" = ?1 AND \"TargetId\" = ?2 AND \"{owner}\" = ?3"),
                        params![i64::from(section), place, who],
                        |row| row.get(0),
                    )
                    .expect("a count");
                count > 0
            };
            let (section_place, place) = places[random.below(places.len() as u64) as usize];
            let exists = if section_place {
                visibility::section_listed(&db, *user, server, place).expect("a check")
            } else {
                visibility::find(&db, *user, server, place, None).expect("a check").is_some()
            };
            let scope = if section_place { authorization::Scope::section(server, place) } else { authorization::Scope::channel(server, place) };
            let invalid = |allow: i64, deny: i64| allow & deny != 0 || (allow | deny) & privileges::ADMINISTRATOR != 0;
            let role_target = all_roles[random.below(all_roles.len() as u64) as usize];
            if !existing("PermissionOverrides", "RoleId", section_place, place, role_target) {
                let (allow, deny) = masks(&mut random);
                let status = if !exists {
                    404
                } else if !manages_roles {
                    403
                } else if invalid(allow, deny) {
                    400
                } else if authorization::can_change_override(&db, *user, &scope, role_target, allow, deny).expect("a check") {
                    200
                } else {
                    403
                };
                probes.push(serde_json::json!({
                    "kind": "override", "section": section_place, "place": place.to_string(), "target": role_target.to_string(),
                    "allow": privileges::to_names(allow), "deny": privileges::to_names(deny), "status": status,
                }));
            }
            let member_target = users[random.below(users.len() as u64) as usize].1;
            if !existing("MemberPermissionOverrides", "UserId", section_place, place, member_target) {
                let (allow, deny) = masks(&mut random);
                let target_is_owner = roles::Member::load(&db, server, member_target).expect("a read").is_some_and(|member| member.is_owner());
                let status = if !exists {
                    404
                } else if !manages_roles {
                    403
                } else if invalid(allow, deny) {
                    400
                } else if target_is_owner {
                    409
                } else if authorization::can_change_member_override(&db, *user, &scope, member_target, allow, deny).expect("a check") {
                    200
                } else {
                    403
                };
                probes.push(serde_json::json!({
                    "kind": "memberOverride", "section": section_place, "place": place.to_string(), "target": member_target.to_string(),
                    "allow": privileges::to_names(allow), "deny": privileges::to_names(deny), "status": status,
                }));
            }
            let line = serde_json::json!({
                "user": name, "server": server.to_string(),
                "permissions": privileges::to_names(permissions),
                "channels": seen_channels, "sections": seen_sections,
                "send": sending, "probes": probes,
            });
            out.push_str(&line.to_string());
            out.push('\n');
        }
    }
    std::fs::write(var("PERMISSIONS_EXPECTED"), out).expect("the answers are written");
}

fn places_for_overrides(sections: &[Id], channels: &[(Id, Option<Id>)]) -> Vec<(bool, Id)> {
    sections.iter().map(|s| (true, *s)).chain(channels.iter().map(|(c, _)| (false, *c))).collect()
}

struct ServerRolesBase {
    role: Role,
}

impl ServerRolesBase {
    fn of(db: &rusqlite::Connection, server: Id, base: Id) -> ServerRolesBase {
        let role = roles::ServerRoles::load(db, server).expect("roles").by_id()[&base];
        ServerRolesBase { role }
    }
}

// ---- What a route does with the rules

mod routes {
    use axum::extract::State;
    use http::Method;
    use rusqlite::params;
    use serde_json::{Value, json};
    use stellarr_api::auth::Caller;
    use stellarr_api::http::{ApiError, ApiResult, Endpoints, Path};
    use stellarr_api::permissions::authorization::{Scope, held_at};
    use stellarr_api::permissions::membership::{ActiveServer, InstanceOwner};
    use stellarr_api::permissions::privileges::{SEND_MESSAGES, CREATE_INVITE};
    use stellarr_api::state::AppState;
    use stellarr_api::values::Id;
    use stellarr_messages::MessageKey;

    use crate::support::{LogCapture, Person, Request, Server};

    fn routes() -> Endpoints {
        Endpoints::new()
            .get("/api/test/active-server", active_server)
            .authorized()
            .get("/api/test/instance", instance)
            .authorized()
            .get("/api/test/channels/{id}/send", send)
            .authorized()
            .get("/api/test/invite", invite)
            .authorized()
            .merge(stellarr_api::accounts::routes())
    }

    async fn active_server(_: Caller, ActiveServer(server): ActiveServer) -> String {
        server.to_string()
    }

    async fn instance(_: Caller, InstanceOwner(server): InstanceOwner) -> String {
        server.to_string()
    }

    /// What a route for a channel does: it is not there for who does not see
    /// it, and a missing permission is the problem that names it.
    async fn send(
        State(state): State<AppState>,
        caller: Caller,
        ActiveServer(server): ActiveServer,
        Path(channel): Path<Id>,
    ) -> ApiResult<String> {
        let held = state
            .database()
            .read(move |db| held_at(db, caller.user, &Scope::channel(server, channel)))
            .await?;
        match held.require(SEND_MESSAGES) {
            Ok(_) => Ok("sent".to_string()),
            Err(refusal) => Err(refusal
                .into_error(state.messages(), MessageKey::SendMessagesDenied, ApiError::not_found())?),
        }
    }

    async fn invite(State(state): State<AppState>, caller: Caller, ActiveServer(server): ActiveServer) -> ApiResult<String> {
        let held = state
            .database()
            .read(move |db| held_at(db, caller.user, &Scope::server(server)))
            .await?;
        match held.has(CREATE_INVITE) {
            true => Ok("made".to_string()),
            false => Err(stellarr_api::permissions::authorization::Refusal::Denied { permission: CREATE_INVITE }
                .into_error(state.messages(), MessageKey::CreateInviteDenied, ApiError::not_found())?),
        }
    }

    struct World {
        server: Server,
        owner: Person,
        jozef: Person,
        stranger: Person,
        general: String,
        hidden: String,
        _log: LogCapture,
    }

    async fn world() -> World {
        let log = LogCapture::start();
        let server = Server::in_process(routes(), &[]).await;
        let mut code = String::new();
        for _ in 0..100 {
            code = log
                .lines()
                .iter()
                .find_map(|line| line.split_once("Setup code: ").map(|(_, rest)| rest.split('.').next().unwrap_or("").to_string()))
                .unwrap_or_default();
            if !code.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let client = server.client();
        let response = client
            .post_json(
                "/api/auth/register",
                &json!({"username": "boss", "displayName": "Boss", "email": "boss@example.test",
                        "password": crate::support::PASSWORD, "setupCode": code}),
            )
            .await;
        assert_eq!(response.status.as_u16(), 201, "{}", response.text());
        let owner = Person { username: "boss".into(), id: response.json::<Value>()["id"].as_str().unwrap().into(), client };
        let jozef = server.register("jozef").await;
        let stranger = server.register("stranger").await;
        let database = server.database();
        let instance: String = database.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        let channels = |name: &str| -> String {
            database.query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Name\" = ?1", [name], |row| row.get(0)).unwrap()
        };
        let general = channels("general");
        // A private channel: view denied to the base role.
        let hidden = Id::random();
        database
            .execute(
                "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, 'secret', 0, 9, '', 0, 0, 0, NULL)",
                params![hidden.to_db_text(), instance],
            )
            .unwrap();
        let base: String = database
            .query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0", [], |row| row.get(0))
            .unwrap();
        database
            .execute(
                "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 16384)",
                params![Id::random().to_db_text(), instance, hidden.to_db_text(), base],
            )
            .unwrap();
        // The stranger leaves the server.
        database
            .execute("DELETE FROM \"ServerMembers\" WHERE \"UserId\" = ?1", [stranger.id.to_uppercase()])
            .unwrap();
        World { server, owner, jozef, stranger, general, hidden: hidden.to_string(), _log: log }
    }

    #[tokio::test]
    async fn a_route_that_takes_the_active_server_answers_a_member_and_refuses_the_rest_as_the_first_server_did() {
        let w = world().await;
        let instance: String = w.server.database().query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        let instance = Id::parse(&instance).unwrap().to_string();
        let named = |value: &str| Request::new(Method::GET, "/api/test/active-server").header("X-Stellarr-Server", value);

        let ok = w.jozef.client.get("/api/test/active-server").await;
        assert_eq!((ok.status.as_u16(), ok.text()), (200, instance.clone()));
        for value in [instance.clone(), instance.replace('-', ""), format!("{{{}}}", instance.to_uppercase()), format!("  {instance} ")] {
            let response = w.jozef.client.send(named(&value)).await;
            assert_eq!((response.status.as_u16(), response.text()), (200, instance.clone()), "{value}");
        }
        let query = w.jozef.client.get(&format!("/api/test/active-server?SERVERID={instance}")).await;
        assert_eq!(query.status.as_u16(), 200);
        // A name that is no id is a 400 and an id of no server of theirs a 403, both with no body.
        let bad = w.jozef.client.send(named("not-an-id")).await;
        assert_eq!((bad.status.as_u16(), bad.body.len()), (400, 0));
        let other = w.jozef.client.send(named("11111111-1111-1111-1111-111111111111")).await;
        assert_eq!((other.status.as_u16(), other.body.len()), (403, 0));
        // A header that is there but empty names nobody, and the query is not asked.
        let empty = w.jozef.client.send(named("").header("X-Unused", "1")).await;
        assert_eq!(empty.status.as_u16(), 200);
        // Somebody who left is a stranger to every server; one who is not signed in is not asked.
        let left = w.stranger.client.get("/api/test/active-server").await;
        assert_eq!((left.status.as_u16(), left.body.len()), (403, 0));
        let nobody = w.server.anonymous().get("/api/test/active-server").await;
        assert_eq!(nobody.status.as_u16(), 401);
    }

    #[tokio::test]
    async fn a_refusal_names_the_permission_in_a_403_problem_and_a_hidden_channel_is_not_there() {
        let w = world().await;

        let allowed = w.jozef.client.get(&format!("/api/test/channels/{}/send", w.general)).await;
        assert_eq!((allowed.status.as_u16(), allowed.text()), (200, "sent".to_string()));
        // Where the first server answers with this problem, byte for byte but for the trace id.
        let denied = w.jozef.client.get("/api/test/invite").await;
        assert_eq!(denied.status.as_u16(), 403);
        assert_eq!(denied.header("content-type"), Some("application/problem+json"));
        let mut body: Value = denied.json();
        assert!(body.as_object_mut().unwrap().remove("traceId").is_some());
        assert_eq!(
            body,
            json!({"type": "https://tools.ietf.org/html/rfc9110#section-15.5.4",
                   "title": "You may not create invites here.", "status": 403, "permission": "createInvite"})
        );
        assert!(denied.text().starts_with("{\"type\":\"https://tools.ietf.org/html/rfc9110#section-15.5.4\",\"title\":\"You may not create invites here.\",\"status\":403,\"permission\":\"createInvite\",\"traceId\":"));
        // The owner holds it, and everything in the hidden channel too.
        assert_eq!(w.owner.client.get("/api/test/invite").await.status, 200);
        assert_eq!(w.owner.client.get(&format!("/api/test/channels/{}/send", w.hidden)).await.status, 200);
        // For somebody who does not see the channel it is the route's own answer for an id that does not exist.
        let hidden = w.jozef.client.get(&format!("/api/test/channels/{}/send", w.hidden)).await;
        let unknown = w.jozef.client.get(&format!("/api/test/channels/{}/send", Id::random())).await;
        assert_eq!((hidden.status.as_u16(), hidden.body.len()), (404, 0));
        assert_eq!((unknown.status.as_u16(), unknown.body.len()), (404, 0));
    }

    #[tokio::test]
    async fn a_channel_where_sending_is_denied_but_view_is_held_says_which_permission_is_missing() {
        let w = world().await;
        let database = w.server.database();
        let base: String = database
            .query_row("SELECT \"Id\" FROM \"ServerRoles\" WHERE \"IsSystem\" = 1 AND \"IsOwner\" = 0", [], |row| row.get(0))
            .unwrap();
        let server: String = database.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        database
            .execute(
                "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 512)",
                params![Id::random().to_db_text(), server, w.general, base],
            )
            .unwrap();

        let denied = w.jozef.client.get(&format!("/api/test/channels/{}/send", w.general)).await;

        assert_eq!(denied.status.as_u16(), 403);
        assert_eq!(denied.json::<Value>()["permission"], "sendMessages");
        assert_eq!(denied.json::<Value>()["title"], "You may not send messages in this channel.");
    }

    #[tokio::test]
    async fn the_instance_panel_is_for_the_owner_of_the_instance_alone_and_says_when_only_a_second_step_is_missing() {
        let w = world().await;

        let owner = w.owner.client.get("/api/test/instance").await;
        let jozef = w.jozef.client.get("/api/test/instance").await;
        assert_eq!(owner.status.as_u16(), 200);
        assert_eq!((jozef.status.as_u16(), jozef.body.len()), (403, 0));

        // Where a second step is asked for, the session itself has to have passed one.
        let instance: String = w.server.database().query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        w.server
            .database()
            .execute(
                "INSERT INTO \"OperationsSettings\" (\"ServerId\",\"PublicUrl\",\"TimeZone\",\"MailingEnabled\",\"MailFromAddress\",\"InviteMailEnabled\",\"RecoveryMailEnabled\",\"AlertMailEnabled\",\"BackupRetentionDays\",\"LogRetentionDays\",\"OrphanObjectRetentionDays\",\"AutomaticCleanupEnabled\",\"UpdatedAt\",\"MfaRequirement\") VALUES (?1,'','UTC',0,'',0,0,0,7,7,7,0,'2026-11-24 00:00:00','cockpit')",
                params![instance],
            )
            .unwrap();
        let owner = w.owner.client.get("/api/test/instance").await;
        assert_eq!(owner.status.as_u16(), 403);
        assert_eq!(owner.header("content-type"), Some("application/problem+json"));
        assert_eq!(owner.json::<Value>()["code"], "cockpit-mfa-required");
        // Somebody who is not the owner is not told about second steps.
        let jozef = w.jozef.client.get("/api/test/instance").await;
        assert_eq!((jozef.status.as_u16(), jozef.body.len()), (403, 0));
        // Once the session has passed one, it opens.
        w.server
            .database()
            .execute("UPDATE \"UserSessions\" SET \"MfaCompletedAt\" = ?1 WHERE \"UserId\" = ?2", params!["2026-11-24 00:00:00", w.owner.id.to_uppercase()])
            .unwrap();
        assert_eq!(w.owner.client.get("/api/test/instance").await.status, 200);
    }
}

// ---- Who hears an event about a private channel

mod audience {
    use std::time::Duration;

    use rusqlite::params;
    use serde_json::Value;
    use stellarr_api::values::{Id, Timestamp};

    use crate::support::{HubClient, Person, Server};

    fn cookie_of(person: &Person) -> String {
        format!("stellarr-dev-session={}", person.client.cookie("stellarr-dev-session").expect("signed in"))
    }

    fn messages(heard: &[Value]) -> Vec<&Value> {
        heard.iter().filter(|record| record["target"] == "MessageCreated").collect()
    }

    #[tokio::test]
    async fn an_event_about_a_private_channel_reaches_a_viewers_connection_on_that_server_and_nobody_elses() {
        let server = Server::start().await;
        let owner = server.found("owner").await;
        let maya = server.register("maya").await;
        let noor = server.register("noor").await;
        let database = server.database();
        let instance: String = database.query_row("SELECT \"Id\" FROM \"Servers\"", [], |row| row.get(0)).unwrap();
        // The first text channel is private to one person: noor's own override denies it.
        let channel: String = database
            .query_row("SELECT \"Id\" FROM \"Channels\" WHERE \"Kind\" = 0 ORDER BY \"Position\" LIMIT 1", [], |row| row.get(0))
            .unwrap();
        database
            .execute(
                "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, 0, ?3, ?4, 0, 16384)",
                params![Id::random().to_db_text(), instance, channel, noor.id.to_uppercase()],
            )
            .unwrap();
        // Maya is on a second server as well, and has a tab open on it.
        let other = Id::random();
        database.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        database
            .execute(
                "INSERT INTO \"ServerMembers\" VALUES (?1, ?2, (SELECT \"RoleId\" FROM \"ServerMembers\" LIMIT 1), NULL, ?3)",
                params![other.to_db_text(), maya.id.to_uppercase(), Timestamp::now()],
            )
            .unwrap();

        let cookie_maya = cookie_of(&maya);
        let mut on_the_server = HubClient::connect_to(server.address(), "", &cookie_maya).await;
        let mut on_the_other = HubClient::connect_to(server.address(), &format!("?serverId={other}"), &cookie_maya).await;
        let mut denied = HubClient::connect_to(server.address(), "", &cookie_of(&noor)).await;
        for client in [&mut on_the_server, &mut on_the_other, &mut denied] {
            client.quiet_for(Duration::from_millis(300)).await;
        }

        // A newcomer is welcomed in the channel, which only some of the members see.
        server.register("zed").await;

        let seen = on_the_server.quiet_for(Duration::from_millis(500)).await;
        let elsewhere = on_the_other.quiet_for(Duration::from_millis(500)).await;
        let refused = denied.quiet_for(Duration::from_millis(500)).await;
        let welcome = messages(&seen);
        assert_eq!(welcome.len(), 1, "{seen:?}");
        assert_eq!(welcome[0]["arguments"][0]["channelId"], channel.to_lowercase());
        assert!(messages(&elsewhere).is_empty(), "{elsewhere:?}");
        assert!(messages(&refused).is_empty(), "{refused:?}");
        let _ = owner;
    }
}
