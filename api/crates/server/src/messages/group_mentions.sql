-- Owners and administrators hold every permission, and their roles were
-- stored before calling on a whole channel was one: they are stored with it
-- now, so that the lists of roles show what such a role holds. Running it
-- again changes nothing.
UPDATE "ServerRoles" SET "Permissions" = "Permissions" | 131072
WHERE "IsOwner" = 1 OR ("Permissions" & 256) <> 0;
