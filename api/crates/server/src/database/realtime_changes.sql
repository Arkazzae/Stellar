-- The journal of changes that can end a live connection or change what it
-- may see, which the API's realtime watcher reads (`realtime/watcher`).
-- A row stays while it is among the newest 1024 or younger than ten minutes,
-- so one large transaction, such as deleting a big server, never prunes rows
-- the watcher has not read yet. A watcher that falls further behind closes
-- its connections, and their clients connect again. Seq never goes back,
-- even when somebody empties the table.
CREATE TABLE IF NOT EXISTS RealtimeChanges (
    Seq INTEGER PRIMARY KEY AUTOINCREMENT,
    Kind TEXT NOT NULL,
    Subject TEXT,
    ServerId TEXT,
    Origin TEXT,
    At INTEGER NOT NULL DEFAULT (CAST(strftime('%s', 'now') AS INTEGER))
);
CREATE TRIGGER IF NOT EXISTS RealtimeChangesTrim
AFTER INSERT ON RealtimeChanges
WHEN NEW.Seq % 128 = 0
BEGIN
    DELETE FROM RealtimeChanges
    WHERE Seq <= NEW.Seq - 1024 AND At < CAST(strftime('%s', 'now') AS INTEGER) - 600;
END;

CREATE TRIGGER IF NOT EXISTS RealtimeUserSessionsInsert
AFTER INSERT ON UserSessions
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('session', NEW.Id, NULL);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeUserSessionsDelete
AFTER DELETE ON UserSessions
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('session', OLD.Id, NULL);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeUserSessionsUpdate
AFTER UPDATE OF Id, UserId, ExpiresAt, AbsoluteExpiresAt, RevokedAt, SecurityVersion, MfaCompletedAt, MfaEvidence ON UserSessions
WHEN OLD.Id IS NOT NEW.Id
  OR OLD.UserId IS NOT NEW.UserId
  OR OLD.ExpiresAt IS NOT NEW.ExpiresAt
  OR OLD.AbsoluteExpiresAt IS NOT NEW.AbsoluteExpiresAt
  OR OLD.RevokedAt IS NOT NEW.RevokedAt
  OR OLD.SecurityVersion IS NOT NEW.SecurityVersion
  OR OLD.MfaCompletedAt IS NOT NEW.MfaCompletedAt
  OR OLD.MfaEvidence IS NOT NEW.MfaEvidence
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('session', NEW.Id, NULL);
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    SELECT 'session', OLD.Id, NULL
    WHERE OLD.Id IS NOT NEW.Id;
END;

CREATE TRIGGER IF NOT EXISTS RealtimeUsersInsert
AFTER INSERT ON Users
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('user', NEW.Id, NULL);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeUsersDelete
AFTER DELETE ON Users
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('user', OLD.Id, NULL);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeUsersUpdate
AFTER UPDATE OF Id, SessionVersion, IsDisabled ON Users
WHEN OLD.Id IS NOT NEW.Id
  OR OLD.SessionVersion IS NOT NEW.SessionVersion
  OR OLD.IsDisabled IS NOT NEW.IsDisabled
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('user', NEW.Id, NULL);
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    SELECT 'user', OLD.Id, NULL
    WHERE OLD.Id IS NOT NEW.Id;
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServerMembersInsert
AFTER INSERT ON ServerMembers
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('member', NEW.UserId, NEW.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServerMembersDelete
AFTER DELETE ON ServerMembers
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('member', OLD.UserId, OLD.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServerMembersUpdate
AFTER UPDATE OF UserId, ServerId, RoleId ON ServerMembers
WHEN OLD.UserId IS NOT NEW.UserId
  OR OLD.ServerId IS NOT NEW.ServerId
  OR OLD.RoleId IS NOT NEW.RoleId
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('member', NEW.UserId, NEW.ServerId);
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    SELECT 'member', OLD.UserId, OLD.ServerId
    WHERE OLD.UserId IS NOT NEW.UserId OR OLD.ServerId IS NOT NEW.ServerId;
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServerMemberRolesInsert
AFTER INSERT ON ServerMemberRoles
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('member', NEW.UserId, NEW.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServerMemberRolesDelete
AFTER DELETE ON ServerMemberRoles
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('member', OLD.UserId, OLD.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServerMemberRolesUpdate
AFTER UPDATE OF UserId, ServerId, RoleId ON ServerMemberRoles
WHEN OLD.UserId IS NOT NEW.UserId
  OR OLD.ServerId IS NOT NEW.ServerId
  OR OLD.RoleId IS NOT NEW.RoleId
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('member', NEW.UserId, NEW.ServerId);
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    SELECT 'member', OLD.UserId, OLD.ServerId
    WHERE OLD.UserId IS NOT NEW.UserId OR OLD.ServerId IS NOT NEW.ServerId;
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServerRolesInsert
AFTER INSERT ON ServerRoles
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES (CASE WHEN NEW.IsSystem AND NOT NEW.IsOwner THEN 'base-role' ELSE 'role' END, NEW.Id, NEW.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServerRolesDelete
AFTER DELETE ON ServerRoles
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES (CASE WHEN OLD.IsSystem AND NOT OLD.IsOwner THEN 'base-role' ELSE 'role' END, OLD.Id, OLD.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServerRolesUpdate
AFTER UPDATE OF Id, ServerId, Permissions, IsOwner, IsSystem, Position ON ServerRoles
WHEN OLD.Id IS NOT NEW.Id
  OR OLD.ServerId IS NOT NEW.ServerId
  OR OLD.Permissions IS NOT NEW.Permissions
  OR OLD.IsOwner IS NOT NEW.IsOwner
  OR OLD.IsSystem IS NOT NEW.IsSystem
  OR OLD.Position IS NOT NEW.Position
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES (CASE WHEN (OLD.IsSystem AND NOT OLD.IsOwner) OR (NEW.IsSystem AND NOT NEW.IsOwner)
                 THEN 'base-role' ELSE 'role' END, NEW.Id, NEW.ServerId);
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    SELECT CASE WHEN (OLD.IsSystem AND NOT OLD.IsOwner) OR (NEW.IsSystem AND NOT NEW.IsOwner)
                THEN 'base-role' ELSE 'role' END, OLD.Id, OLD.ServerId
    WHERE OLD.Id IS NOT NEW.Id OR OLD.ServerId IS NOT NEW.ServerId;
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServersInsert
AFTER INSERT ON Servers
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('instance', NULL, NEW.Id);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServersDelete
AFTER DELETE ON Servers
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('instance', NULL, OLD.Id);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeServersUpdate
AFTER UPDATE OF Id, CreatedAt ON Servers
WHEN OLD.Id IS NOT NEW.Id
  OR OLD.CreatedAt IS NOT NEW.CreatedAt
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('instance', NULL, NEW.Id);
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    SELECT 'instance', NULL, OLD.Id
    WHERE OLD.Id IS NOT NEW.Id;
END;

CREATE TRIGGER IF NOT EXISTS RealtimeOperationsSettingsInsert
AFTER INSERT ON OperationsSettings
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('policy', NULL, NEW.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeOperationsSettingsDelete
AFTER DELETE ON OperationsSettings
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('policy', NULL, OLD.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeOperationsSettingsUpdate
AFTER UPDATE OF ServerId, MfaRequirement ON OperationsSettings
WHEN OLD.ServerId IS NOT NEW.ServerId
  OR OLD.MfaRequirement IS NOT NEW.MfaRequirement
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('policy', NULL, NEW.ServerId);
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    SELECT 'policy', NULL, OLD.ServerId
    WHERE OLD.ServerId IS NOT NEW.ServerId;
END;

CREATE TRIGGER IF NOT EXISTS RealtimePermissionOverridesInsert
AFTER INSERT ON PermissionOverrides
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('access', NULL, NEW.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimePermissionOverridesDelete
AFTER DELETE ON PermissionOverrides
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('access', NULL, OLD.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimePermissionOverridesUpdate
AFTER UPDATE OF ServerId, RoleId, TargetId, TargetType, AllowMask, DenyMask ON PermissionOverrides
WHEN OLD.ServerId IS NOT NEW.ServerId
  OR OLD.RoleId IS NOT NEW.RoleId
  OR OLD.TargetId IS NOT NEW.TargetId
  OR OLD.TargetType IS NOT NEW.TargetType
  OR OLD.AllowMask IS NOT NEW.AllowMask
  OR OLD.DenyMask IS NOT NEW.DenyMask
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('access', NULL, NEW.ServerId);
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    SELECT 'access', NULL, OLD.ServerId
    WHERE OLD.ServerId IS NOT NEW.ServerId;
END;

CREATE TRIGGER IF NOT EXISTS RealtimeMemberPermissionOverridesInsert
AFTER INSERT ON MemberPermissionOverrides
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('access', NEW.UserId, NEW.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeMemberPermissionOverridesDelete
AFTER DELETE ON MemberPermissionOverrides
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('access', OLD.UserId, OLD.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeMemberPermissionOverridesUpdate
AFTER UPDATE OF ServerId, UserId, TargetId, TargetType, AllowMask, DenyMask ON MemberPermissionOverrides
WHEN OLD.ServerId IS NOT NEW.ServerId
  OR OLD.UserId IS NOT NEW.UserId
  OR OLD.TargetId IS NOT NEW.TargetId
  OR OLD.TargetType IS NOT NEW.TargetType
  OR OLD.AllowMask IS NOT NEW.AllowMask
  OR OLD.DenyMask IS NOT NEW.DenyMask
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('access', NEW.UserId, NEW.ServerId);
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    SELECT 'access', OLD.UserId, OLD.ServerId
    WHERE OLD.UserId IS NOT NEW.UserId OR OLD.ServerId IS NOT NEW.ServerId;
END;

CREATE TRIGGER IF NOT EXISTS RealtimeChannelsInsert
AFTER INSERT ON Channels
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('channel', NEW.Id, NEW.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeChannelsDelete
AFTER DELETE ON Channels
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('channel', OLD.Id, OLD.ServerId);
END;

CREATE TRIGGER IF NOT EXISTS RealtimeChannelsUpdate
AFTER UPDATE OF Id, ServerId, SectionId, Kind ON Channels
WHEN OLD.Id IS NOT NEW.Id
  OR OLD.ServerId IS NOT NEW.ServerId
  OR OLD.SectionId IS NOT NEW.SectionId
  OR OLD.Kind IS NOT NEW.Kind
BEGIN
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    VALUES ('channel', NEW.Id, NEW.ServerId);
    INSERT INTO RealtimeChanges(Kind, Subject, ServerId)
    SELECT 'channel', OLD.Id, OLD.ServerId
    WHERE OLD.Id IS NOT NEW.Id OR OLD.ServerId IS NOT NEW.ServerId;
END;
