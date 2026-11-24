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
    managers: Vec<String>,
    members: Vec<String>,
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

/// Names that test what is trimmed: characters some libraries count as
/// white space and others do not.
fn random_name(random: &mut Random) -> String {
    let edge = ['\u{1c}', '\u{1f}', '\u{85}', '\u{a0}', '\u{180e}', '\u{200b}', '\u{2028}', '\u{3000}', '\u{feff}', '\t', '\u{b}', ' '];
    match random.below(4) {
        0 => {
            let wrap = *random.pick(&edge);
            format!("{wrap}Edge{wrap}")
        }
        1 => (*random.pick(&["Alpha", "Bravo", "Role 0", "Role 1", "Member", "  Padded  ", "", "Alpha"])).to_string(),
        2 => "x".repeat(random.pick(&[63, 64, 65]).to_owned()),
        _ => format!("N{}", random.below(5)),
    }
}

/// A request with something wrong or unusual in its shape: no body, a member
/// left out or null, an id in another spelling, no content type.
fn odd_step(random: &mut Random, world: &World, actor: &str, person: Id, role: Id) -> Value {
    let spelled = |id: Id, random: &mut Random| -> String {
        match random.below(3) {
            0 => id.to_string().to_uppercase(),
            1 => id.to_string().replace('-', ""),
            _ => id.to_string(),
        }
    };
    // The first server read an id in a path in any spelling, and a body only in
    // the one with hyphens (or, for a list, with none).
    let in_path = |id: Id, random: &mut Random| -> String {
        match random.below(5) {
            0 => id.to_string().to_uppercase(),
            1 => id.to_string().replace('-', ""),
            2 => format!("%7B{id}%7D"),
            3 => format!("%28{id}%29"),
            _ => id.to_string(),
        }
    };
    let (target, named) = (in_path(person, random), in_path(role, random));
    let loose = spelled(role, random);
    let mut one = match random.below(14) {
        0 => step(actor, "POST", format!("/api/admin/members/{target}/ban"), None),
        1 => step(actor, "POST", format!("/api/admin/members/{person}/ban"), Some(json!({}))),
        2 => step(actor, "POST", format!("/api/admin/members/{person}/ban"), Some(json!({ "reason": null }))),
        3 => step(actor, "POST", format!("/api/admin/members/{person}/ban"), Some(json!({ "reason": "r".repeat(*random.pick(&[499, 500, 501])) }))),
        4 => step(actor, "PATCH", format!("/api/admin/members/{person}/role"), Some(json!({}))),
        5 => step(actor, "PATCH", format!("/api/admin/members/{person}/role"), Some(json!({ "roleId": null }))),
        6 => step(actor, "PATCH", format!("/api/admin/members/{target}/role"), Some(json!({ "roleId": loose }))),
        7 => step(actor, "PUT", format!("/api/admin/members/{person}/roles"), Some(json!({}))),
        8 => step(actor, "PUT", format!("/api/admin/members/{person}/roles"), Some(json!({ "roleIds": null }))),
        9 => step(actor, "PUT", format!("/api/admin/members/{target}/roles"), Some(json!({ "roleIds": [loose, role.to_string()] }))),
        10 => step(actor, "PATCH", format!("/api/admin/roles/{named}"), Some(json!({}))),
        11 => step(actor, "POST", "/api/admin/roles".into(), Some(json!({ "name": "NoPermissions", "color": "#fff" }))),
        12 => step(actor, "PUT", "/api/admin/roles/order".into(), Some(json!({}))),
        _ => step(actor, "DELETE", format!("/api/admin/roles/{named}"), None),
    };
    // Now and then the request does not say what its body is.
    if random.chance(6) {
        one["noContentType"] = json!(true);
    }
    // A body that cannot be read is answered before membership is asked in the
    // first server, and after it here: strangers are not sent one.
    one["odd"] = json!(true);
    let _ = world;
    one
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
    if random.chance(9) {
        return odd_step(random, world, &actor, person, any_role);
    }
    match random.below(22) {
        0 => step(&actor, "GET", "/api/admin/roles".into(), None),
        1 => step(&actor, "GET", "/api/admin/members".into(), None),
        2 => step(&actor, "GET", "/api/admin/bans".into(), None),
        3 | 4 => {
            let name = random_name(random);
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
    // The last two accounts are disabled: targets, never actors.
    let ghosts: Vec<String> = users.iter().rev().take(2).map(|(name, _)| name.clone()).collect();
    for (_, id) in users.iter().rev().take(2) {
        db.execute("UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Id\" = ?1", [id]).expect("a disabled account");
    }

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
        let mut others: Vec<(Id, String)> = (0..2 + random.below(5))
            .map(|n| {
                let name = format!("Role {n}");
                (role(&name, 1 + random.below(4) as i64, mask(&mut random), false, false), name)
            })
            .collect();
        // Roles that hold the four powers the routes ask for first, so that
        // the decisions behind them (rank, hidden grants, the fallback) are
        // what is tested and not only the first refusal.
        let gate = privileges::MANAGE_ROLES | privileges::MANAGE_MEMBERS | privileges::MODERATE_MEMBERS | privileges::MANAGE_BANS | privileges::VIEW_CHANNEL;
        let manager_roles: Vec<(Id, String)> = (0..1 + random.below(2))
            .map(|n| {
                let name = format!("Manager {n}");
                let extra = if random.chance(2) { mask(&mut random) & !ADMINISTRATOR } else { 0 };
                (role(&name, 1 + n as i64 * (1 + random.below(2) as i64), gate | extra, false, false), name)
            })
            .collect();
        others.extend(manager_roles.iter().cloned());
        // A second system role that ranks above the base role.
        if random.chance(12) {
            let name = "Second system".to_string();
            others.push((role(&name, 4, mask(&mut random), true, false), name));
        }
        // An override whose channel or section is gone.
        let orphan_role = others[random.below(others.len() as u64) as usize].0;
        db.execute(
            "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![Id::random(), server, i64::from(random.chance(2)), Id::random(), orphan_role, mask(&mut random) & !ADMINISTRATOR],
        )
        .expect("an orphaned override");
        let no_base = random.chance(20);
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
                let mut held: Vec<Role> = others.iter().filter(|_| random.chance(3)).map(|(id, _)| by_id[id]).collect();
                if random.chance(4) {
                    held.push(by_id[&manager_roles[0].0]);
                }
                if no_base && held.is_empty() {
                    held.push(by_id[&others[0].0]);
                }
                held
            };
            let top = roles::top(&held, base);
            ownership::admit(&db, server, *user, top.id, top.id == base_role, (position == 0).then_some(1), Timestamp::now()).expect("a member");
            ownership::set_roles(&db, server, *user, &held, &base).expect("held roles");
            people.push((name.clone(), *user));
        }
        if no_base {
            // A server whose base role is gone: what the routes say then.
            db.execute("DELETE FROM \"ServerRoles\" WHERE \"Id\" = ?1", [base_role]).expect("no base role");
            // A second system role then is the base role. The first server puts the base
            // role last when it starts (a repair this server, which opens a file as it is,
            // does not make), so the file is left as that repair would leave it.
            db.execute("UPDATE \"ServerRoles\" SET \"Position\" = 8 WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0", [server]).expect("a last base role");
        }
        // Everybody the plan may name, members or not.
        let everybody: Vec<(String, Id)> = users.clone();
        let all_roles: Vec<(Id, String)> = std::iter::once((owner_role, "Owner".to_string()))
            .chain(others.iter().cloned())
            .chain((!no_base).then(|| (base_role, "Member".to_string())))
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
        let by_name: std::collections::HashMap<Id, String> = users.iter().map(|(n, i)| (*i, n.clone())).collect();
        let manager_ids: Vec<Id> = manager_roles.iter().map(|(id, _)| *id).collect();
        let managers: Vec<String> = people
            .iter()
            .filter(|(_, user)| {
                roles::Member::load(&db, server, *user)
                    .ok()
                    .flatten()
                    .is_some_and(|member| {
                        roles::held_of_member(&db, &member).map(|(held, _)| held.iter().any(|r| manager_ids.contains(&r.id))).unwrap_or(false)
                    })
            })
            .map(|(_, user)| by_name[user].clone())
            .collect();
        let world = World {
            managers,
            members: people.iter().map(|(name, _)| name.clone()).collect(),
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
            // Half the time the actor is somebody with the powers to be asked.
            if random.chance(2) && !world.managers.is_empty() {
                one["actor"] = json!(random.pick(&world.managers));
            }
            // A stranger is an actor now and then; the rest are members.
            if random.chance(12) && one.get("odd").is_none() {
                let stranger = world.people.iter().find(|(name, _)| !member_names.contains(name) && !ghosts.contains(name));
                if let Some((name, _)) = stranger {
                    one["actor"] = json!(name);
                }
            }
            if ghosts.iter().any(|ghost| one["actor"] == json!(ghost)) {
                one["actor"] = json!(users[0].0);
            }
            if one.get("odd").is_some() && !member_names.iter().any(|name| one["actor"] == json!(name)) {
                one["actor"] = json!(users[0].0);
            }
            let actor = one["actor"].as_str().unwrap_or_default().to_string();
            one["class"] = json!(if actor == users[0].0 {
                "owner"
            } else if world.managers.contains(&actor) {
                "manager"
            } else if world.members.contains(&actor) {
                "other"
            } else {
                "stranger"
            });
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

/// One world of the same shape for every pair of requests, each with one
/// request: the person who may, and the nearest who may not, for every row of
/// the delegation table. The people are, in the order of the users file:
/// owner, `adm` (Admin), `sen` (Senior), `peer` (Senior), `jun` (Junior),
/// `plain` (base role only), `big` (Powerful), `stranger`.
#[test]
#[ignore = "run by tests/roles-shared-database.sh with ROLES_PAIRS=1"]
fn arrange_pairs_of_requests_that_may_and_may_not_in_a_shared_database() {
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
    assert!(users.len() >= 8, "the pairs need eight people");
    let gate = privileges::MANAGE_ROLES | privileges::MANAGE_MEMBERS | privileges::MODERATE_MEMBERS | privileges::MANAGE_BANS | privileges::VIEW_CHANNEL;
    let ordinary = privileges::SEND_MESSAGES | privileges::VIEW_CHANNEL;
    let (admin_mask, senior_mask, big_mask) = (
        gate | privileges::MANAGE_CHANNELS | ordinary,
        gate | ordinary,
        gate | ordinary | privileges::MANAGE_SERVER,
    );

    // (who, how it is asked, whether it is a change of roles, of an override...)
    struct Variant {
        actor: usize,
        method: &'static str,
        /// A path with `{r:Name}` for a role, `{u:n}` for the n-th person, `{c1}`, `{c2}`, `{s1}`.
        path: String,
        body: Value,
    }
    let v = |actor: usize, method: &'static str, path: &str, body: Value| Variant {
        actor,
        method,
        path: path.to_string(),
        body,
    };
    let (adm, sen, peer, jun, plain, big, stranger) = (1, 2, 3, 4, 5, 6, 7);
    let names = |mask: i64| privileges::to_names(mask);
    let mut variants: Vec<Variant> = Vec::new();
    for place in ["/api/admin/channels/{c1}/permissions", "/api/admin/sections/{s1}/permissions"] {
        variants.extend([
            v(sen, "PUT", &format!("{place}/{{r:Junior}}"), json!({ "allow": names(privileges::SEND_MESSAGES), "deny": [] })),
            v(sen, "PUT", &format!("{place}/{{r:Admin}}"), json!({ "allow": names(privileges::SEND_MESSAGES), "deny": [] })),
            v(sen, "PUT", &format!("{place}/{{r:Senior}}"), json!({ "allow": names(privileges::SEND_MESSAGES), "deny": [] })),
            v(sen, "PUT", &format!("{place}/{{r:Junior}}"), json!({ "allow": names(privileges::MANAGE_SERVER & !ADMINISTRATOR), "deny": [] })),
            v(sen, "PUT", &format!("{place}/{{r:Junior}}"), json!({ "allow": [], "deny": names(privileges::MANAGE_CHANNELS) })),
            v(adm, "PUT", &format!("{place}/{{r:Junior}}"), json!({ "allow": [], "deny": names(privileges::MANAGE_CHANNELS) })),
            v(sen, "DELETE", &format!("{place}/{{r:Junior}}"), Value::Null),
            v(sen, "DELETE", &format!("{place}/{{r:Spare}}"), Value::Null),
            v(adm, "DELETE", &format!("{place}/{{r:Spare}}"), Value::Null),
            v(0, "DELETE", &format!("{place}/{{r:Owner}}"), Value::Null),
            v(0, "PUT", &format!("{place}/{{r:Owner}}"), json!({ "allow": [], "deny": [] })),
            v(sen, "PUT", &format!("{place}/members/{{u:{jun}}}"), json!({ "allow": names(privileges::SEND_MESSAGES), "deny": [] })),
            v(sen, "PUT", &format!("{place}/members/{{u:{peer}}}"), json!({ "allow": names(privileges::SEND_MESSAGES), "deny": [] })),
            v(sen, "PUT", &format!("{place}/members/{{u:{sen}}}"), json!({ "allow": names(privileges::SEND_MESSAGES), "deny": [] })),
            v(sen, "PUT", &format!("{place}/members/{{u:0}}"), json!({ "allow": names(privileges::SEND_MESSAGES), "deny": [] })),
            v(sen, "PUT", &format!("{place}/members/{{u:{big}}}"), json!({ "allow": names(privileges::SEND_MESSAGES), "deny": [] })),
            v(adm, "PUT", &format!("{place}/members/{{u:{big}}}"), json!({ "allow": names(privileges::SEND_MESSAGES), "deny": [] })),
            v(adm, "PUT", &format!("{place}/members/{{u:{plain}}}"), json!({ "allow": names(privileges::MANAGE_SERVER), "deny": [] })),
            v(sen, "DELETE", &format!("{place}/members/{{u:{jun}}}"), Value::Null),
            v(sen, "DELETE", &format!("{place}/members/{{u:{peer}}}"), Value::Null),
            v(adm, "DELETE", &format!("{place}/members/{{u:{peer}}}"), Value::Null),
            v(sen, "DELETE", &format!("{place}/members/{{u:{stranger}}}"), Value::Null),
        ]);
    }
    let everyone = |order: &[&str]| json!({ "roleIds": order.iter().map(|n| format!("{{r:{n}}}")).collect::<Vec<_>>() });
    variants.extend([
        v(sen, "POST", "/api/admin/roles", json!({ "name": "New", "color": "#fff", "permissions": names(ordinary) })),
        v(sen, "POST", "/api/admin/roles", json!({ "name": "New", "color": "#fff", "permissions": names(privileges::MANAGE_SERVER) })),
        v(sen, "POST", "/api/admin/roles", json!({ "name": "New", "color": "#fff", "permissions": names(privileges::MANAGE_CHANNELS) })),
        v(adm, "POST", "/api/admin/roles", json!({ "name": "New", "color": "#fff", "permissions": names(privileges::MANAGE_CHANNELS) })),
        v(jun, "POST", "/api/admin/roles", json!({ "name": "New", "color": "#fff", "permissions": [] })),
        v(adm, "PUT", "/api/admin/roles/order", everyone(&["Admin", "Senior", "Powerful", "Spare", "Junior"])),
        v(sen, "PUT", "/api/admin/roles/order", everyone(&["Admin", "Senior", "Powerful", "Spare", "Junior"])),
        v(sen, "PUT", "/api/admin/roles/order", everyone(&["Senior", "Admin", "Powerful", "Junior", "Spare"])),
        v(sen, "PUT", "/api/admin/roles/order", everyone(&["Admin", "Powerful", "Senior", "Junior", "Spare"])),
        v(adm, "PUT", "/api/admin/roles/order", everyone(&["Senior", "Admin", "Powerful", "Junior", "Spare"])),
        v(adm, "PATCH", "/api/admin/roles/{r:Junior}", json!({ "name": "Renamed" })),
        v(sen, "PATCH", "/api/admin/roles/{r:Junior}", json!({ "permissions": names(ordinary | privileges::MANAGE_CHANNELS) })),
        v(sen, "PATCH", "/api/admin/roles/{r:Junior}", json!({ "permissions": names(ordinary) })),
        v(sen, "PATCH", "/api/admin/roles/{r:Senior}", json!({ "name": "Renamed" })),
        v(sen, "PATCH", "/api/admin/roles/{r:Powerful}", json!({ "name": "Renamed" })),
        v(adm, "PATCH", "/api/admin/roles/{r:Powerful}", json!({ "name": "Renamed" })),
        v(adm, "PATCH", "/api/admin/roles/{r:Powerful}", json!({ "permissions": names(ordinary) })),
        v(adm, "PATCH", "/api/admin/roles/{r:Admin}", json!({ "name": "Renamed" })),
        v(0, "PATCH", "/api/admin/roles/{r:Owner}", json!({ "name": "Renamed" })),
        v(adm, "PATCH", "/api/admin/roles/{r:Member}", json!({ "name": "Renamed" })),
        v(adm, "DELETE", "/api/admin/roles/{r:Junior}", Value::Null),
        v(sen, "DELETE", "/api/admin/roles/{r:Junior}", Value::Null),
        v(sen, "DELETE", "/api/admin/roles/{r:Senior}", Value::Null),
        v(adm, "DELETE", "/api/admin/roles/{r:Powerful}", Value::Null),
        v(adm, "DELETE", "/api/admin/roles/{r:Member}", Value::Null),
        v(0, "DELETE", "/api/admin/roles/{r:Owner}", Value::Null),
        v(adm, "PATCH", &format!("/api/admin/members/{{u:{plain}}}/role"), json!({ "roleId": "{r:Junior}" })),
        v(sen, "PATCH", &format!("/api/admin/members/{{u:{plain}}}/role"), json!({ "roleId": "{r:Junior}" })),
        v(sen, "PATCH", &format!("/api/admin/members/{{u:{plain}}}/role"), json!({ "roleId": "{r:Powerful}" })),
        v(adm, "PATCH", &format!("/api/admin/members/{{u:{plain}}}/role"), json!({ "roleId": "{r:Powerful}" })),
        v(sen, "PATCH", &format!("/api/admin/members/{{u:{peer}}}/role"), json!({ "roleId": "{r:Junior}" })),
        v(adm, "PATCH", &format!("/api/admin/members/{{u:{adm}}}/role"), json!({ "roleId": "{r:Junior}" })),
        v(adm, "PATCH", "/api/admin/members/{u:0}/role", json!({ "roleId": "{r:Junior}" })),
        v(adm, "PATCH", &format!("/api/admin/members/{{u:{plain}}}/role"), json!({ "roleId": "{r:Owner}" })),
        v(adm, "PATCH", &format!("/api/admin/members/{{u:{big}}}/role"), json!({ "roleId": "{r:Member}" })),
        v(adm, "PATCH", &format!("/api/admin/members/{{u:{jun}}}/role"), json!({ "roleId": "{r:Member}" })),
        v(adm, "PUT", &format!("/api/admin/members/{{u:{plain}}}/roles"), json!({ "roleIds": ["{r:Junior}", "{r:Spare}"] })),
        v(sen, "PUT", &format!("/api/admin/members/{{u:{plain}}}/roles"), json!({ "roleIds": ["{r:Junior}", "{r:Powerful}"] })),
        v(adm, "PUT", &format!("/api/admin/members/{{u:{jun}}}/roles"), json!({ "roleIds": [] })),
        v(sen, "PUT", &format!("/api/admin/members/{{u:{big}}}/roles"), json!({ "roleIds": [] })),
        v(adm, "PUT", &format!("/api/admin/members/{{u:{big}}}/roles"), json!({ "roleIds": [] })),
        v(sen, "PUT", &format!("/api/admin/members/{{u:{sen}}}/roles"), json!({ "roleIds": [] })),
        v(adm, "PUT", &format!("/api/admin/members/{{u:{jun}}}/roles"), json!({ "roleIds": ["{r:Junior}"] })),
        v(adm, "PUT", &format!("/api/admin/members/{{u:{plain}}}/roles"), json!({ "roleIds": ["{r:Owner}"] })),
    ]);
    for actor in [adm, sen, jun] {
        for target in [0, adm, sen, peer, jun, plain, big, stranger] {
            variants.push(v(actor, "DELETE", &format!("/api/admin/members/{{u:{target}}}"), Value::Null));
            variants.push(v(actor, "POST", &format!("/api/admin/members/{{u:{target}}}/ban"), json!({ "reason": "pair" })));
        }
    }
    variants.push(v(adm, "GET", "/api/admin/bans", Value::Null));
    variants.push(v(jun, "GET", "/api/admin/bans", Value::Null));
    variants.push(v(sen, "DELETE", &format!("/api/admin/bans/{{u:{stranger}}}"), Value::Null));

    let mut out = String::new();
    for (number, variant) in variants.iter().enumerate() {
        let server = Id::random();
        db.execute(
            "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, ?2, '', 0, ?3, 0, 0)",
            params![server, format!("Pair {number}"), Timestamp::now()],
        )
        .expect("a server");
        let mut ids = std::collections::HashMap::new();
        let mut role = |name: &str, position: i64, permissions: i64, system: bool, owner: bool| {
            let id = Id::random();
            db.execute(
                "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, ?3, '#777777', ?4, ?5, ?6, ?7)",
                params![id, server, name, permissions, position, system, owner],
            )
            .expect("a role");
            ids.insert(name.to_string(), id);
        };
        role("Owner", 0, privileges::ALL, true, true);
        role("Admin", 1, admin_mask, false, false);
        role("Senior", 2, senior_mask, false, false);
        role("Powerful", 2, big_mask, false, false);
        role("Junior", 3, ordinary, false, false);
        role("Spare", 3, privileges::SEND_MESSAGES, false, false);
        role("Member", 7, privileges::BASE_ROLE_DEFAULTS, true, false);
        let by_id = ServerRoles::load(&db, server).expect("roles").by_id();
        let base = by_id[&ids["Member"]];
        let held_by_person: [Option<&str>; 8] = [Some("Owner"), Some("Admin"), Some("Senior"), Some("Senior"), Some("Junior"), None, Some("Powerful"), None];
        for (position, held) in held_by_person.iter().enumerate() {
            if position == 7 {
                continue;
            }
            let held: Vec<Role> = held.iter().map(|name| by_id[&ids[*name]]).collect();
            let top = roles::top(&held, base);
            ownership::admit(&db, server, users[position].1, top.id, top.id == base.id, (position == 0).then_some(1), Timestamp::now()).expect("a member");
            ownership::set_roles(&db, server, users[position].1, &held, &base).expect("held roles");
        }
        let section = Id::random();
        db.execute("INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, 'Section', 0)", params![section, server]).expect("a section");
        let (c1, c2) = (Id::random(), Id::random());
        for (id, name, in_section) in [(c1, "one", None), (c2, "two", Some(section))] {
            db.execute(
                "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, ?3, 0, 0, '', 0, 0, 0, ?4)",
                params![id, server, name, in_section],
            )
            .expect("a channel");
        }
        // Overrides to be cleared or to count as hidden grants: Junior is denied one
        // power sen lacks and one it holds, Spare is allowed one sen lacks, at both places.
        for (target_type, place) in [(0_i64, c1), (1, section)] {
            for (role_name, allow, deny) in [("Junior", 0, privileges::MANAGE_CHANNELS | privileges::SEND_MESSAGES), ("Spare", privileges::MANAGE_CHANNELS, 0)] {
                db.execute(
                    "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![Id::random(), server, target_type, place, ids[role_name], allow, deny],
                )
                .expect("an override");
            }
            db.execute(
                "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                params![Id::random(), server, target_type, place, users[jun].1, privileges::MANAGE_CHANNELS],
            )
            .expect("an override of a member");
        }
        // Words in the path and body become the ids of this world.
        let fill = |text: &str| -> String {
            let mut text = text.replace("{c1}", &c1.to_string()).replace("{s1}", &section.to_string());
            for (name, id) in &ids {
                text = text.replace(&format!("{{r:{name}}}"), &id.to_string());
            }
            for (index, (_, id)) in users.iter().enumerate() {
                text = text.replace(&format!("{{u:{index}}}"), &id.to_string());
            }
            text
        };
        let body: Value = if variant.body.is_null() {
            Value::Null
        } else {
            serde_json::from_str(&fill(&variant.body.to_string())).expect("a body")
        };
        let class = ["owner", "adm", "sen", "peer", "jun", "plain", "big", "stranger"][variant.actor];
        let step = json!({
            "actor": users[variant.actor].0, "method": variant.method, "path": fill(&variant.path), "body": body,
            "class": class,
        });
        let known: Vec<String> = users.iter().map(|(_, id)| id.to_string()).chain(ids.values().map(Id::to_string)).chain([c1.to_string(), section.to_string(), server.to_string()]).collect();
        let mut labels = serde_json::Map::new();
        for (name, id) in &users {
            labels.insert(id.to_string(), json!(name));
        }
        for (name, id) in &ids {
            labels.insert(id.to_string(), json!(format!("role {name}")));
        }
        out.push_str(&json!({
            "server": server.to_string(), "known": known, "labels": labels,
            "members": users.iter().take(7).map(|(name, _)| name.clone()).collect::<Vec<_>>(), "steps": [step],
        })
        .to_string());
        out.push('\n');
    }
    std::fs::write(var("ROLES_PLAN"), out).expect("the plan is written");
}

/// The second step, on the instance's own server and on another. Roles are
/// made on the instance server, and the people hold them: `person1` (Admin),
/// `person2` (Admin), `person3` (Admin), `person4` (Junior), `person5`
/// (Senior). The script then gives `person1`, `person4` and `person5` a second
/// step that was never passed in their sessions, `person2` one passed a minute
/// ago, and `person3` none, before either server is asked. `person1` also owns
/// a second server, where the step must not be asked for.
///
/// `ROLES_STALE_PLAN` is where the plan is written, beside the variables of
/// the other arrangements.
#[test]
#[ignore = "run by tests/roles-shared-database.sh"]
fn arrange_a_second_step_on_the_instance_server_and_on_another() {
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
    let server: Id = db
        .query_row("SELECT \"Id\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1", [], |row| row.get(0))
        .expect("the instance server");
    let gate = privileges::MANAGE_ROLES | privileges::MANAGE_MEMBERS | privileges::MODERATE_MEMBERS | privileges::MANAGE_BANS | privileges::VIEW_CHANNEL;
    let ordinary = privileges::SEND_MESSAGES | privileges::VIEW_CHANNEL;
    let mut ids = std::collections::HashMap::new();
    let mut role = |name: &str, position: i64, permissions: i64| {
        let id = Id::random();
        db.execute(
            "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, ?3, '#777777', ?4, ?5, 0, 0)",
            params![id, server, name, permissions, position],
        )
        .expect("a role");
        ids.insert(name.to_string(), id);
    };
    role("Admin", 1, gate | privileges::MANAGE_CHANNELS | ordinary);
    role("Senior", 2, gate | ordinary);
    role("Junior", 3, ordinary);
    role("Spare", 4, privileges::SEND_MESSAGES);
    db.execute("UPDATE \"ServerRoles\" SET \"Position\" = 9 WHERE \"ServerId\" = ?1 AND \"IsSystem\" = 1 AND \"IsOwner\" = 0", [server]).expect("the base role last");
    let all = ServerRoles::load(&db, server).expect("roles");
    let base = all.base.expect("a base role");
    let by_id = all.by_id();
    let owner_role = all.roles.iter().find(|role| role.is_owner).expect("an owner role").id;
    for (name, held) in [("person1", "Admin"), ("person2", "Admin"), ("person3", "Admin"), ("person4", "Junior"), ("person5", "Senior")] {
        let user = users.iter().find(|(user, _)| user == name).expect("a person").1;
        ownership::set_roles(&db, server, user, &[by_id[&ids[held]]], &base).expect("held roles");
    }
    let section = Id::random();
    let channel = Id::random();
    db.execute("INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, 'Section', 0)", params![section, server]).expect("a section");
    db.execute(
        "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, 'stale-room', 0, 99, '', 0, 0, 0, NULL)",
        params![channel, server],
    )
    .expect("a channel");

    // A second server, owned by person1.
    let other = Id::random();
    db.execute(
        "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, 'Other', '', 0, ?2, 0, 0)",
        params![other, Timestamp::now()],
    )
    .expect("a server");
    let other_owner = Id::random();
    let other_base = Id::random();
    for (id, name, position, permissions, system, owner) in [(other_owner, "Owner", 0, privileges::ALL, true, true), (other_base, "Member", 5, privileges::BASE_ROLE_DEFAULTS, true, false)] {
        db.execute(
            "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, ?3, '#777777', ?4, ?5, ?6, ?7)",
            params![id, other, name, permissions, position, system, owner],
        )
        .expect("a role");
    }
    let person1 = users.iter().find(|(user, _)| user == "person1").expect("a person").1;
    let other_roles = ServerRoles::load(&db, other).expect("roles");
    ownership::admit(&db, other, person1, other_owner, false, Some(1), Timestamp::now()).expect("a member");
    ownership::set_roles(&db, other, person1, &[other_roles.by_id()[&other_owner]], &other_roles.base.expect("a base role")).expect("held roles");
    let _ = owner_role;

    let id = |name: &str| ids[name].to_string();
    let user = |name: &str| users.iter().find(|(user, _)| user == name).expect("a person").1.to_string();
    let mut steps: Vec<Value> = Vec::new();
    let order: Vec<String> = ["Admin", "Senior", "Junior", "Spare"].iter().map(|name| id(name)).collect();
    let routes = |actor: &str, steps: &mut Vec<Value>, class: &str| {
        let plain = user("person7");
        let mut all = vec![
            ("POST", "/api/admin/roles".to_string(), Some(json!({ "name": format!("New {actor}"), "color": "#fff", "permissions": [] }))),
            ("PUT", "/api/admin/roles/order".to_string(), Some(json!({ "roleIds": order }))),
            ("PATCH", format!("/api/admin/roles/{}", id("Junior")), Some(json!({ "name": format!("Junior {actor}") }))),
            ("DELETE", format!("/api/admin/roles/{}", id("Spare")), None),
            ("PATCH", format!("/api/admin/members/{plain}/role"), Some(json!({ "roleId": id("Junior") }))),
            ("PUT", format!("/api/admin/members/{plain}/roles"), Some(json!({ "roleIds": [id("Junior")] }))),
        ];
        for (collection, place) in [("channels", channel), ("sections", section)] {
            let base = format!("/api/admin/{collection}/{place}/permissions");
            let masks = Some(json!({ "allow": ["sendMessages"], "deny": [] }));
            all.push(("PUT", format!("{base}/{}", id("Junior")), masks.clone()));
            all.push(("DELETE", format!("{base}/{}", id("Junior")), None));
            all.push(("PUT", format!("{base}/members/{plain}"), masks));
            all.push(("DELETE", format!("{base}/members/{plain}"), None));
        }
        for (method, path, body) in all {
            steps.push(json!({ "actor": actor, "method": method, "path": path, "body": body, "class": class }));
        }
    };
    // Stale accounts first: they change nothing.
    routes("person1", &mut steps, "stale admin");
    routes("person4", &mut steps, "stale, no power");
    routes("person5", &mut steps, "stale, a senior");
    // A stale admin who fails something else as well.
    let stale = "person1";
    let more = |steps: &mut Vec<Value>, method: &str, path: String, body: Option<Value>, extra: Value| {
        let mut step = json!({ "actor": stale, "method": method, "path": path, "body": body, "class": "stale, two refusals" });
        for (key, value) in extra.as_object().expect("an object") {
            step[key] = value.clone();
        }
        steps.push(step);
    };
    more(&mut steps, "PATCH", format!("/api/admin/roles/{}", id("Admin")), Some(json!({ "name": "X" })), json!({}));
    more(&mut steps, "PATCH", format!("/api/admin/roles/{}", Id::random()), Some(json!({})), json!({}));
    more(&mut steps, "POST", "/api/admin/roles".into(), Some(json!({ "name": "", "color": "" })), json!({}));
    more(&mut steps, "PUT", "/api/admin/roles/order".into(), Some(json!({})), json!({}));
    more(&mut steps, "POST", "/api/admin/roles".into(), None, json!({ "raw": "{", "odd": true }));
    more(&mut steps, "POST", "/api/admin/roles".into(), None, json!({ "raw": "", "odd": true }));
    more(&mut steps, "PATCH", format!("/api/admin/members/{}/role", user("boss")), Some(json!({ "roleId": id("Junior") })), json!({}));
    more(&mut steps, "PATCH", format!("/api/admin/members/{}/role", user("person7")), Some(json!({ "roleId": null })), json!({ "odd": true }));
    more(&mut steps, "PATCH", format!("/api/admin/members/{}/role", user("person7")), Some(json!({ "roleId": id("Junior") })), json!({ "server": Id::random().to_string() }));
    // On the second server, which is not the instance's own, the step is not asked for.
    more(&mut steps, "POST", "/api/admin/roles".into(), Some(json!({ "name": "Elsewhere", "color": "#fff", "permissions": [] })), json!({ "server": other.to_string() }));
    more(&mut steps, "GET", "/api/admin/roles".into(), None, json!({}));
    // A fresh step, and none to pass.
    routes("person2", &mut steps, "fresh admin");
    routes("person3", &mut steps, "admin with no second step");

    let known: Vec<String> = users.iter().map(|(_, id)| id.to_string()).chain(ids.values().map(Id::to_string)).chain([channel.to_string(), section.to_string(), server.to_string(), other.to_string()]).collect();
    let mut labels = serde_json::Map::new();
    for (name, id) in &users {
        labels.insert(id.to_string(), json!(name));
    }
    for (name, id) in &ids {
        labels.insert(id.to_string(), json!(format!("role {name}")));
    }
    // The instance's own server is `server`; the plan's requests name it by default.
    let line = json!({ "server": server.to_string(), "known": known, "labels": labels, "members": [], "steps": steps });
    std::fs::write(var("ROLES_STALE_PLAN"), format!("{line}\n")).expect("the plan is written");
}
