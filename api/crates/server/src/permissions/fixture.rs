//! A server to arrange rows on, for the tests of the rules: roles, people,
//! channels and overrides written straight into a database of its own.

use std::sync::{Arc, Mutex};

use rusqlite::{Connection, params};

use super::privileges::*;
use super::visibility::ChannelKind;
use crate::database::Database;
use crate::testing::{TempDir, new_database};
use crate::values::{Id, Timestamp};

/// The roles, people and places of one server. Positions run `owner 0`,
/// `moderator 1`, `base 2`, so a lower value ranks higher.
pub struct World {
    _folder: TempDir,
    pub database: Database,
    pub db: Connection,
    pub server: Id,
    pub owner_role: Id,
    pub moderator_role: Id,
    pub base_role: Id,
    pub maja: Id,
    pub ada: Id,
    pub ilya: Id,
    pub nils: Id,
    pub jo: Id,
    pub general: Id,
    pub projects: Id,
    pub voice: Id,
    statements: Arc<Mutex<Vec<String>>>,
}

pub const MODERATOR: i64 = MANAGE_MESSAGES | MODERATE_MEMBERS | MANAGE_BANS;

impl World {
    pub fn new() -> World {
        let (folder, database) = new_database();
        let db = Connection::open(database.path()).expect("a connection");
        db.pragma_update(None, "foreign_keys", true)
            .expect("foreign keys");
        let mut world = World {
            _folder: folder,
            database,
            db,
            server: Id::random(),
            owner_role: Id::random(),
            moderator_role: Id::random(),
            base_role: Id::random(),
            maja: Id::random(),
            ada: Id::random(),
            ilya: Id::random(),
            nils: Id::random(),
            jo: Id::random(),
            general: Id::random(),
            projects: Id::random(),
            voice: Id::random(),
            statements: Arc::default(),
        };
        world.arrange();
        world
    }

    fn arrange(&mut self) {
        let server = self.server;
        self.db
            .execute(
                "INSERT INTO \"Servers\" (\"Id\", \"Name\", \"Description\", \"IsPublic\", \"CreatedAt\", \"SoundboardEveryoneCanPlay\", \"SoundboardRateLimited\") VALUES (?1, 'Test', '', 0, ?2, 0, 0)",
                params![server, Timestamp::now()],
            )
            .expect("a server");
        self.insert_role(self.owner_role, "Owner", 0, ALL, true, true);
        self.insert_role(self.moderator_role, "Moderator", 1, MODERATOR, false, false);
        self.insert_role(self.base_role, "Member", 2, BASE_ROLE_DEFAULTS, true, false);
        for (user, name, role) in [
            (self.maja, "maja", self.owner_role),
            (self.ada, "ada", self.moderator_role),
            (self.ilya, "ilya", self.base_role),
            (self.nils, "nils", self.base_role),
            (self.jo, "jo", self.base_role),
        ] {
            self.insert_user(user, name);
            self.admit(user, role);
        }
        self.insert_channel(self.general, None, ChannelKind::Text, "general");
        self.insert_channel(self.projects, None, ChannelKind::Text, "projects");
        self.insert_channel(self.voice, None, ChannelKind::Voice, "rooms");
    }

    pub fn insert_role(
        &self,
        id: Id,
        name: &str,
        position: i64,
        permissions: i64,
        system: bool,
        owner: bool,
    ) {
        self.db
            .execute(
                "INSERT INTO \"ServerRoles\" (\"Id\", \"ServerId\", \"Name\", \"Color\", \"Permissions\", \"Position\", \"IsSystem\", \"IsOwner\") VALUES (?1, ?2, ?3, '#777777', ?4, ?5, ?6, ?7)",
                params![id, self.server, name, permissions, position, system, owner],
            )
            .expect("a role");
    }

    pub fn role(&self, name: &str, position: i64, permissions: i64) -> Id {
        let id = Id::random();
        self.insert_role(
            id,
            &format!("{name}-{id}"),
            position,
            permissions,
            false,
            false,
        );
        id
    }

    pub fn insert_user(&self, id: Id, name: &str) {
        self.db
            .execute(
                "INSERT INTO \"Users\" (\"Id\", \"Username\", \"NormalizedUsername\", \"DisplayName\", \"IsDisabled\", \"SessionVersion\", \"SecurityRecoveryRequired\", \"AvatarColor\", \"CreatedAt\") VALUES (?1, ?2, ?3, ?4, 0, 0, 0, '#000', ?5)",
                params![id, name, name.to_uppercase(), format!("{name} Display"), Timestamp::now()],
            )
            .expect("a user");
    }

    pub fn user(&self, name: &str) -> Id {
        let id = Id::random();
        self.insert_user(id, name);
        id
    }

    /// A membership whose top role is `role`, with the role stored as held
    /// when it is not the base role, as every writer does.
    pub fn admit(&self, user: Id, role: Id) {
        let owner = role == self.owner_role;
        super::ownership::admit(
            &self.db,
            self.server,
            user,
            role,
            role == self.base_role,
            owner.then_some(1),
            Timestamp::now(),
        )
        .expect("a membership");
    }

    /// A member who holds exactly these roles besides the base role, stored
    /// as the roles of the member and as the top role.
    pub fn member_of(&self, user: Id, roles: &[Id]) {
        let base = super::roles::find_member_role(&self.db, self.server)
            .expect("a read")
            .expect("a base role");
        let held: Vec<super::roles::Role> = roles
            .iter()
            .map(|id| {
                super::roles::ServerRoles::load(&self.db, self.server)
                    .expect("roles")
                    .by_id()[id]
            })
            .collect();
        let top = super::roles::top(&held, base);
        super::ownership::admit(
            &self.db,
            self.server,
            user,
            top.id,
            top.id == base.id,
            None,
            Timestamp::now(),
        )
        .expect("a membership");
        super::ownership::set_roles(&self.db, self.server, user, &held, &base).expect("roles");
    }

    pub fn insert_channel(&self, id: Id, section: Option<Id>, kind: ChannelKind, name: &str) {
        self.db
            .execute(
                "INSERT INTO \"Channels\" (\"Id\", \"ServerId\", \"Name\", \"Kind\", \"Position\", \"Topic\", \"SlowModeSeconds\", \"HideAfterInactivityHours\", \"UserLimit\", \"SectionId\") VALUES (?1, ?2, ?3, ?4, 0, '', 0, 0, 0, ?5)",
                params![id, self.server, name, kind, section],
            )
            .expect("a channel");
    }

    pub fn channel(&self, section: Option<Id>) -> Id {
        let id = Id::random();
        self.insert_channel(id, section, ChannelKind::Text, &format!("c-{id}"));
        id
    }

    pub fn section(&self) -> Id {
        let id = Id::random();
        self.db
            .execute(
                "INSERT INTO \"Sections\" (\"Id\", \"ServerId\", \"Name\", \"Position\") VALUES (?1, ?2, ?3, 0)",
                params![id, self.server, format!("s-{id}")],
            )
            .expect("a section");
        id
    }

    /// An override of a role, at a channel (`false`) or a section (`true`).
    pub fn override_of(&self, section: bool, target: Id, role: Id, allow: i64, deny: i64) {
        self.db
            .execute(
                "INSERT INTO \"PermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"RoleId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![Id::random(), self.server, i64::from(section), target, role, allow, deny],
            )
            .expect("an override");
    }

    pub fn member_override(&self, section: bool, target: Id, user: Id, allow: i64, deny: i64) {
        self.db
            .execute(
                "INSERT INTO \"MemberPermissionOverrides\" (\"Id\", \"ServerId\", \"TargetType\", \"TargetId\", \"UserId\", \"AllowMask\", \"DenyMask\") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![Id::random(), self.server, i64::from(section), target, user, allow, deny],
            )
            .expect("an override of a member");
    }

    pub fn set_permissions(&self, role: Id, permissions: i64) {
        self.db
            .execute(
                "UPDATE \"ServerRoles\" SET \"Permissions\" = ?1 WHERE \"Id\" = ?2",
                params![permissions, role],
            )
            .expect("a role");
    }

    pub fn set_position(&self, role: Id, position: i64) {
        self.db
            .execute(
                "UPDATE \"ServerRoles\" SET \"Position\" = ?1 WHERE \"Id\" = ?2",
                params![position, role],
            )
            .expect("a role");
    }

    pub fn disable(&self, user: Id) {
        self.db
            .execute(
                "UPDATE \"Users\" SET \"IsDisabled\" = 1 WHERE \"Id\" = ?1",
                [user],
            )
            .expect("a user");
    }

    pub fn deny_view_to_base(&self, target: Id, section: bool) {
        self.override_of(section, target, self.base_role, NONE, VIEW_CHANNEL);
    }

    pub fn allow_view_to(&self, target: Id, section: bool, role: Id) {
        self.override_of(section, target, role, VIEW_CHANNEL, NONE);
    }

    /// Starts writing down every statement the connection runs.
    pub fn count_statements(&mut self) {
        let log = Arc::clone(&self.statements);
        log.lock().expect("log").clear();
        self.db
            .trace_v2(
                rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
                Some(move |event: rusqlite::trace::TraceEvent<'_>| {
                    if let rusqlite::trace::TraceEvent::Stmt(_, sql) = event {
                        log.lock().expect("log").push(sql.to_string());
                    }
                }),
            )
            .expect("a trace");
    }

    /// The statements run since the last call.
    pub fn statements(&self) -> Vec<String> {
        std::mem::take(&mut *self.statements.lock().expect("log"))
    }
}
