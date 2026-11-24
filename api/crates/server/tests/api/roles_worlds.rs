//! Servers of random shape arranged in a copy of the first server's database,
//! and a plan of requests for each, for `tests/roles-shared-database.sh`.
//!
//! The script sends the same plan to this server and to the first one, each
//! on its own copy of the file, and compares every answer and the rows the two
//! leave. Each world is one server of its own, so one file carries hundreds of
//! independent cases.
//!
//! `ROLES_DB` is the database file, `ROLES_USERS` a file of `name id` lines of
//! accounts that exist in it (the first owns every server), `ROLES_PLAN` where
//! the plan is written, `ROLES_WORLDS` how many servers to make, `ROLES_STEPS`
//! how many requests each gets and `ROLES_SEED` which shapes.

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use stellarr_api::permissions::privileges::{self, ADMINISTRATOR, ALL};
use stellarr_api::permissions::roles::{self, Role, ServerRoles};
use stellarr_api::permissions::ownership;
use stellarr_api::values::{Id, Timestamp};

/// A small generator that is the same on every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// Permissions that the delegation rules treat in their own way, with the
/// ones that decide who may moderate.
const KINDS: [i64; 9] = [
    privileges::VIEW_CHANNEL,
    privileges::SEND_MESSAGES,
    privileges::MANAGE_CHANNELS,
    privileges::MANAGE_ROLES,
    privileges::MANAGE_ROLES,
    privileges::MANAGE_MEMBERS,
    privileges::MODERATE_MEMBERS,
    privileges::MANAGE_BANS,
    privileges::CONNECT,
];

fn mask(random: &mut Random) -> i64 {
    let ordinary = KINDS.iter().filter(|_| random.chance(3)).fold(0, |all, bit| all | bit);
    if random.chance(16) { ordinary | ADMINISTRATOR } else { ordinary }
}

/// What a member of a server can be asked to do, by a person of the server.
struct World {
    roles: Vec<(Id, String)>,
    owner_role: Id,
    base_role: Id,
    people: Vec<(String, Id)>,
    places: Vec<(bool, Id)>,
}

fn names(mask: i64) -> Vec<&'static str> {
    privileges::to_names(mask)
}

/// A request: who sends it, and what.
fn step(actor: &str, method: &str, path: String, body: Option<Value>) -> Value {
    json!({ "actor": actor, "method": method, "path": path, "body": body })
}

fn random_step(random: &mut Random, world: &World) -> Value {
    // Mostly the people of the server who have powers; the owner now and then.
    let actor = if random.chance(3) { &world.people[0].0 } else { &random.pick(&world.people).0 }.clone();
    let person = random.pick(&world.people).1;
    let any_role = random.pick(&world.roles).0;
    let movable = |random: &mut Random| -> Id {
        loop {
            let (id, _) = random.pick(&world.roles);
            if *id != world.owner_role && *id != world.base_role || random.chance(8) {
                return *id;
            }
        }
    };
    let (section, place) = *random.pick(&world.places);
    let collection = if section { "sections" } else { "channels" };
    let overrides = format!("/api/admin/{collection}/{place}/permissions");
    let masks = |random: &mut Random| -> Value {
        let allow = mask(random);
        let deny = if random.chance(8) { mask(random) } else { mask(random) & !allow };
        json!({ "allow": names(allow), "deny": names(deny) })
    };
    match random.below(22) {
        0 => step(&actor, "GET", "/api/admin/roles".into(), None),
        1 => step(&actor, "GET", "/api/admin/members".into(), None),
        2 => step(&actor, "GET", "/api/admin/bans".into(), None),
        3 | 4 => {
            let name = *random.pick(&["Alpha", "Bravo", "Role 0", "Role 1", "Member", "  Padded  ", "", "Alpha"]);
            let permissions = if random.chance(6) { json!(["nonsense"]) } else { json!(names(mask(random))) };
            step(&actor, "POST", "/api/admin/roles".into(), Some(json!({
                "name": name, "color": if random.chance(9) { "" } else { "#7c6ff0" }, "permissions": permissions,
            })))
        }
        5 | 6 => {
            let mut order: Vec<Id> = world
                .roles
                .iter()
                .map(|(id, _)| *id)
                .filter(|id| *id != world.owner_role && *id != world.base_role)
                .collect();
            for index in (1..order.len()).rev() {
                let other = random.below(index as u64 + 1) as usize;
                order.swap(index, other);
            }
            if random.chance(12) {
                order.pop();
            }
            step(&actor, "PUT", "/api/admin/roles/order".into(), Some(json!({ "roleIds": order })))
        }
        7 | 8 => {
            let mut body = serde_json::Map::new();
            if random.chance(2) {
                body.insert("name".into(), json!(*random.pick(&["Renamed", "Alpha", "Role 0", "Role 1", "Role 2", " Trim "])));
            }
            if random.chance(3) {
                body.insert("color".into(), json!("#112233"));
            }
            if random.chance(2) {
                body.insert("permissions".into(), json!(names(mask(random))));
            }
            step(&actor, "PATCH", format!("/api/admin/roles/{}", movable(random)), Some(Value::Object(body)))
        }
        9 => step(&actor, "DELETE", format!("/api/admin/roles/{}", movable(random)), None),
        10 | 11 => step(&actor, "PATCH", format!("/api/admin/members/{person}/role"), Some(json!({ "roleId": any_role }))),
        12 | 13 => {
            let held: Vec<Id> = world.roles.iter().map(|(id, _)| *id).filter(|_| random.chance(3)).collect();
            step(&actor, "PUT", format!("/api/admin/members/{person}/roles"), Some(json!({ "roleIds": held })))
        }
        14 => step(&actor, "PUT", format!("{overrides}/{any_role}"), Some(masks(random))),
        15 => step(&actor, "DELETE", format!("{overrides}/{any_role}"), None),
        16 => step(&actor, "PUT", format!("{overrides}/members/{person}"), Some(masks(random))),
        17 => step(&actor, "DELETE", format!("{overrides}/members/{person}"), None),
        18 => step(&actor, "GET", format!("{overrides}{}", if random.chance(2) { "/members" } else { "" }), None),
        19 => {
            let reason = *random.pick(&["", "spam", "  trimmed  ", "x"]);
            step(&actor, "POST", format!("/api/admin/members/{person}/ban"), Some(json!({ "reason": reason })))
        }
        20 => step(&actor, "DELETE", format!("/api/admin/members/{person}"), None),
        _ => step(&actor, "DELETE", format!("/api/admin/bans/{person}"), None),
    }
}

#[test]
#[ignore = "run by tests/roles-shared-database.sh"]
fn arrange_worlds_in_a_shared_database_and_plan_requests_for_each() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
    let db = Connection::open(var("ROLES_DB")).expect("the database");
    db.pragma_update(None, "foreign_keys", true).expect("foreign keys");
    let users: Vec<(String, Id)> = std::fs::read_to_string(var("ROLES_USERS"))
        .expect("the users")
        .lines()
        .filter_map(|line| {
            let (name, id) = line.split_once(' ')?;
            Some((name.to_string(), Id::parse(id)?))
        })
        .collect();
    let worlds: u64 = var("ROLES_WORLDS").parse().expect("a count");
    let steps: u64 = var("ROLES_STEPS").parse().expect("a count");
    let mut random = Random(var("ROLES_SEED").parse::<u64>().expect("a seed") | 1);
    let mut out = String::new();

    for number in 0..worlds {
        let server = Id::random();
        db.execute(
            "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, ?2, '', 0, ?3, 0, 0)",
            params![server, format!("World {number}"), Timestamp::now()],
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
        let base_permissions = match random.below(5) {
            0 => privileges::SEND_MESSAGES,
            1 => 0,
            _ => privileges::BASE_ROLE_DEFAULTS,
        };
        let others: Vec<(Id, String)> = (0..2 + random.below(5))
            .map(|n| {
                let name = format!("Role {n}");
                (role(&name, 1 + random.below(4) as i64, mask(&mut random), false, false), name)
            })
            .collect();
        let base_role = role("Member", 7, base_permissions, true, false);
        let base: Role = ServerRoles::load(&db, server).expect("roles").by_id()[&base_role];

        let sections: Vec<Id> = (0..random.below(4)).map(|_| Id::random()).collect();
        for (n, section) in sections.iter().enumerate() {
            db.execute(
                "INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, ?3, ?4)",
                params![section, server, format!("Section {n}"), n as i64],
            )
            .expect("a section");
        }
        let mut channels: Vec<(Id, Option<Id>)> = Vec::new();
        for n in 0..2 + random.below(7) {
            let id = Id::random();
            let section = (!sections.is_empty() && random.chance(2)).then(|| sections[random.below(sections.len() as u64) as usize]);
            db.execute(
                "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, ?3, ?4, ?5, '', 0, 0, 0, ?6)",
                params![id, server, format!("channel-{n}"), i64::from(random.chance(4)), n as i64, section],
            )
            .expect("a channel");
            channels.push((id, section));
        }

        // People: the first owns the server, most of the others are members, a
        // few are banned and a few are strangers.
        let mut people = Vec::new();
        for (position, (name, user)) in users.iter().enumerate() {
            if position != 0 && random.chance(8) {
                continue;
            }
            if position != 0 && random.chance(10) {
                db.execute(
                    "INSERT INTO \"ServerBans\" (\"ServerId\", \"UserId\", \"BannedById\", \"Reason\", \"BannedAt\") VALUES (?1, ?2, ?3, 'arranged', ?4)",
                    params![server, user, users[0].1, Timestamp::now()],
                )
                .expect("a ban");
                continue;
            }
            let held: Vec<Role> = if position == 0 {
                vec![ServerRoles::load(&db, server).expect("roles").by_id()[&owner_role]]
            } else {
                let by_id = ServerRoles::load(&db, server).expect("roles").by_id();
                others.iter().filter(|_| random.chance(3)).map(|(id, _)| by_id[id]).collect()
            };
            let top = roles::top(&held, base);
            ownership::admit(&db, server, *user, top.id, top.id == base_role, (position == 0).then_some(1), Timestamp::now()).expect("a member");
            ownership::set_roles(&db, server, *user, &held, &base).expect("held roles");
            people.push((name.clone(), *user));
        }
        // Everybody the plan may name, members or not.
        let everybody: Vec<(String, Id)> = users.clone();
        let all_roles: Vec<(Id, String)> = std::iter::once((owner_role, "Owner".to_string()))
            .chain(others.iter().cloned())
            .chain([(base_role, "Member".to_string())])
            .collect();
        let places: Vec<(bool, Id)> = sections.iter().map(|s| (true, *s)).chain(channels.iter().map(|(c, _)| (false, *c))).collect();
        for (section, place) in &places {
            for (role_id, _) in &all_roles {
                if *role_id != owner_role && random.chance(3) {
                    let allow = mask(&mut random);
                    let deny = mask(&mut random) & !allow;
                    db.execute(
                        "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![Id::random(), server, i64::from(*section), place, role_id, allow & !ADMINISTRATOR, deny & !ADMINISTRATOR],
                    )
                    .expect("an override");
                }
            }
            for (_, user) in people.iter().skip(1) {
                if random.chance(6) {
                    let allow = mask(&mut random) & !ADMINISTRATOR & ALL;
                    let deny = mask(&mut random) & !allow & !ADMINISTRATOR;
                    db.execute(
                        "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![Id::random(), server, i64::from(*section), place, user, allow, deny],
                    )
                    .expect("an override of a member");
                }
            }
        }

        // Only members can send anything: the others ask what a stranger is told.
        let world = World {
            roles: all_roles.clone(),
            owner_role,
            base_role,
            people: everybody,
            places,
        };
        let member_names: Vec<String> = people.iter().map(|(name, _)| name.clone()).collect();
        let mut plan = Vec::new();
        for _ in 0..steps {
            let mut one = random_step(&mut random, &world);
            // A stranger is an actor now and then; the rest are members.
            if random.chance(12) {
                let stranger = world.people.iter().find(|(name, _)| !member_names.contains(name));
                if let Some((name, _)) = stranger {
                    one["actor"] = json!(name);
                }
            }
            plan.push(one);
        }
        let known: Vec<String> = world
            .people
            .iter()
            .map(|(_, id)| id.to_string())
            .chain(world.roles.iter().map(|(id, _)| id.to_string()))
            .chain(world.places.iter().map(|(_, id)| id.to_string()))
            .chain([server.to_string()])
            .collect();
        // Names for the ids, so that a transcript reads the same on both sides.
        let mut labels = serde_json::Map::new();
        for (name, id) in &world.people {
            labels.insert(id.to_string(), json!(name));
        }
        for (id, name) in &world.roles {
            labels.insert(id.to_string(), json!(format!("role {name}")));
        }
        for (index, (section, id)) in world.places.iter().enumerate() {
            labels.insert(id.to_string(), json!(format!("{} {index}", if *section { "section" } else { "channel" })));
        }
        out.push_str(&json!({
            "server": server.to_string(), "known": known, "labels": labels, "members": member_names, "steps": plan,
        })
        .to_string());
        out.push('\n');
    }
    std::fs::write(var("ROLES_PLAN"), out).expect("the plan is written");
}
