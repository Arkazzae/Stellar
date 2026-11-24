-- maya: two methods, codes, a trusted device, a pending handoff, two identities, events.
INSERT INTO MfaMethods (Id,UserId,Type,Name,CreatedAt,ConfirmedAt,LastUsedAt,ProtectedSecret,EmailAddress,LastTotpStep,RevokedAt) VALUES
 ('11111111-1111-4111-8111-111111111111','@MAYA@','totp','Phone','2026-11-10 09:00:00','2026-11-10 09:01:00','2026-11-20 08:00:00.5','secret-should-not-show',NULL,5,NULL),
 ('22222222-2222-4222-8222-222222222222','@MAYA@','email','Mail','2026-11-11 09:00:00','2026-11-11 09:01:00',NULL,NULL,'MAYA@EXAMPLE.TEST',NULL,NULL),
 ('33333333-3333-4333-8333-333333333333','@MAYA@','totp','Old','2026-11-01 09:00:00',NULL,NULL,'x',NULL,NULL,NULL),
 ('44444444-4444-4444-8444-444444444444','@MAYA@','totp','Gone','2026-10-01 09:00:00','2026-10-01 09:01:00',NULL,'x',NULL,NULL,'2026-10-02 09:00:00');
INSERT INTO RecoveryCodes (Id,UserId,BatchId,CodeHash,CreatedAt,UsedAt) VALUES
 ('a1111111-1111-4111-8111-111111111111','@MAYA@','b1111111-1111-4111-8111-111111111111','h1','2026-11-10 09:02:00',NULL),
 ('a2222222-2222-4222-8222-222222222222','@MAYA@','b1111111-1111-4111-8111-111111111111','h2','2026-11-10 09:02:00',NULL),
 ('a3333333-3333-4333-8333-333333333333','@MAYA@','b1111111-1111-4111-8111-111111111111','h3','2026-11-10 09:02:00','2026-11-12 10:00:00');
INSERT INTO TrustedDevices (Id,UserId,TokenHash,Name,Device,IpAddress,CreatedAt,ExpiresAt,LastUsedAt,RevokedAt) VALUES
 ('c1111111-1111-4111-8111-111111111111','@MAYA@','t1','Laptop','Firefox','10.0.0.5','2026-11-15 09:00:00','2030-01-01 00:00:00','2026-11-20 08:00:00',NULL),
 ('c2222222-2222-4222-8222-222222222222','@MAYA@','t2','Old laptop','Chrome','10.0.0.6','2026-01-15 09:00:00','2026-02-01 00:00:00',NULL,NULL);
INSERT INTO DesktopBrowserLogins (Id,SecretHash,ApprovalTokenHash,ApprovalUserId,ApprovalSessionId,UserId,ApprovedSessionId,PrimarySource,MfaCompletedAt,MfaEvidence,SecurityVersion,CreatedAt,ExpiresAt,ApprovedAt,RedeemedAt) VALUES
 ('d1111111-1111-4111-8111-111111111111','s1',NULL,NULL,NULL,'@MAYA@',NULL,'password',NULL,NULL,1,'2026-11-24 10:00:00','2030-01-01 00:00:00','2026-11-24 10:00:10',NULL);
INSERT INTO ExternalIdentities (Id,UserId,Issuer,Subject,ProviderId,CreatedAt) VALUES
 ('e1111111-1111-4111-8111-111111111111','@MAYA@','https://id.example.test','sub-1','work','2026-11-12 09:00:00'),
 ('e2222222-2222-4222-8222-222222222222','@MAYA@','https://other.example.test','sub-2',NULL,'2026-11-13 09:00:00'),
 ('e3333333-3333-4333-8333-333333333333','@OWNER@','https://id.example.test','sub-3','lab','2026-11-13 09:00:00');
INSERT INTO SecurityEvents (Id,UserId,Kind,Detail,IpAddress,Device,CreatedAt) VALUES
 ('f1111111-1111-4111-8111-111111111111','@MAYA@','login-succeeded','password','10.0.0.5','Firefox','2026-11-20 08:00:00'),
 ('f2222222-2222-4222-8222-222222222222','@MAYA@','method-added','totp Phone','10.0.0.5','Firefox','2026-11-10 09:01:00.25');
-- a disabled account and a placeholder of imported messages.
INSERT INTO Users (Id,Username,NormalizedUsername,DisplayName,PasswordHash,Email,NormalizedEmail,ProviderVerifiedEmail,IsDisabled,SessionVersion,SecurityRecoveryRequired,AvatarColor,CreatedAt,Origin) VALUES
 ('99999999-9999-4999-8999-999999999999','dora','DORA','Dora',NULL,NULL,NULL,NULL,1,3,0,'c','2026-11-05 09:00:00',NULL),
 ('88888888-8888-4888-8888-888888888888','imported-1','IMPORTED-1','Imported One',NULL,NULL,NULL,NULL,0,1,0,'c','2026-11-06 09:00:00','discord');
-- an open handoff of the owner that is not approved.
INSERT INTO DesktopBrowserLogins (Id,SecretHash,UserId,SecurityVersion,CreatedAt,ExpiresAt,ApprovedAt,RedeemedAt) VALUES
 ('d2222222-2222-4222-8222-222222222222','s2','@OWNER@',1,'2026-11-24 10:00:00','2030-01-01 00:00:00',NULL,NULL);
