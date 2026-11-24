CREATE TABLE IF NOT EXISTS "Channels" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_Channels" PRIMARY KEY,
    "ServerId" TEXT NOT NULL,
    "Name" TEXT NOT NULL,
    "Kind" INTEGER NOT NULL,
    "Position" INTEGER NOT NULL,
    "Topic" TEXT NOT NULL DEFAULT '',
    "SlowModeSeconds" INTEGER NOT NULL,
    "HideAfterInactivityHours" INTEGER NOT NULL,
    "UserLimit" INTEGER NOT NULL,
    "SectionId" TEXT NULL
);
CREATE TABLE IF NOT EXISTS "IdentityProviderSettings" (
    "ProviderId" TEXT NOT NULL CONSTRAINT "PK_IdentityProviderSettings" PRIMARY KEY,
    "SignUpAllowed" INTEGER NOT NULL,
    "UpdatedAt" TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS "ImageAttachments" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_ImageAttachments" PRIMARY KEY,
    "UploadedById" TEXT NOT NULL,
    "ObjectKey" TEXT NOT NULL,
    "FileName" TEXT NOT NULL,
    "ContentType" TEXT NOT NULL,
    "SizeBytes" INTEGER NOT NULL,
    "Width" INTEGER NULL,
    "Height" INTEGER NULL,
    "DurationSeconds" REAL NULL,
    "IsVoiceMessage" INTEGER NOT NULL DEFAULT 0,
    "PosterObjectKey" TEXT NULL,
    "PosterContentType" TEXT NULL,
    "PosterSizeBytes" INTEGER NULL,
    "PreviewObjectKey" TEXT NULL,
    "PreviewContentType" TEXT NULL,
    "PreviewSizeBytes" INTEGER NULL,
    "Placeholder" TEXT NULL,
    "IsEncrypted" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS "InstanceSecrets" (
    "Name" TEXT NOT NULL CONSTRAINT "PK_InstanceSecrets" PRIMARY KEY,
    "ProtectedValue" TEXT NOT NULL,
    "UpdatedAt" TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS "MessageChanges" (
    "Sequence" INTEGER NOT NULL CONSTRAINT "PK_MessageChanges" PRIMARY KEY AUTOINCREMENT,
    "ChannelId" TEXT NOT NULL,
    "MessageId" TEXT NOT NULL,
    "Kind" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL
);
CREATE TABLE sqlite_sequence(name,seq);
CREATE TABLE IF NOT EXISTS "ObjectDeletions" (
    "ObjectKey" TEXT NOT NULL CONSTRAINT "PK_ObjectDeletions" PRIMARY KEY,
    "QueuedAt" TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS "OperatorAuditEntries" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_OperatorAuditEntries" PRIMARY KEY,
    "Actor" TEXT NOT NULL,
    "Operation" TEXT NOT NULL,
    "Target" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS "PasskeySignIns" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_PasskeySignIns" PRIMARY KEY,
    "SecretHash" TEXT NOT NULL,
    "Options" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL,
    "ConsumedAt" TEXT NULL
);
CREATE TABLE IF NOT EXISTS "PendingExternalIdentities" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_PendingExternalIdentities" PRIMARY KEY,
    "SecretHash" TEXT NOT NULL,
    "ProviderId" TEXT NOT NULL,
    "Issuer" TEXT NOT NULL,
    "Subject" TEXT NOT NULL,
    "Email" TEXT NULL,
    "EmailVerified" INTEGER NOT NULL,
    "DisplayName" TEXT NULL,
    "PreferredUsername" TEXT NULL,
    "Amr" TEXT NOT NULL,
    "AuthTime" TEXT NULL,
    "ShortLived" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS "Servers" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_Servers" PRIMARY KEY,
    "Name" TEXT NOT NULL,
    "Description" TEXT NOT NULL DEFAULT '',
    "IsPublic" INTEGER NOT NULL DEFAULT 0,
    "IconUrl" TEXT NULL,
    "CreatedAt" TEXT NOT NULL,
    "SoundboardEveryoneCanPlay" INTEGER NOT NULL,
    "SoundboardRateLimited" INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS "Users" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_Users" PRIMARY KEY,
    "Username" TEXT NOT NULL,
    "NormalizedUsername" TEXT NOT NULL,
    "DisplayName" TEXT NOT NULL,
    "PasswordHash" TEXT NULL,
    "Email" TEXT NULL,
    "NormalizedEmail" TEXT NULL,
    "ProviderVerifiedEmail" TEXT NULL,
    "IsDisabled" INTEGER NOT NULL,
    "SessionVersion" INTEGER NOT NULL,
    "SecurityRecoveryRequired" INTEGER NOT NULL,
    "AvatarColor" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "AvatarUrl" TEXT NULL,
    "BannerUrl" TEXT NULL,
    "WallpaperUrl" TEXT NULL,
    "CustomStatus" TEXT NULL,
    "Bio" TEXT NULL,
    "Pronouns" TEXT NULL,
    "Origin" TEXT NULL
);
CREATE TABLE IF NOT EXISTS "OperationsSettings" (
    "ServerId" TEXT NOT NULL CONSTRAINT "PK_OperationsSettings" PRIMARY KEY,
    "PublicUrl" TEXT NOT NULL,
    "TimeZone" TEXT NOT NULL,
    "MailingEnabled" INTEGER NOT NULL,
    "MailFromAddress" TEXT NOT NULL,
    "MailSmtpHost" TEXT NOT NULL DEFAULT '',
    "MailSmtpPort" INTEGER NOT NULL DEFAULT 587,
    "MailSmtpUsername" TEXT NOT NULL DEFAULT '',
    "MailSmtpSecurity" TEXT NOT NULL DEFAULT 'starttls',
    "MailResendEnabled" INTEGER NULL,
    "GiphyEnabled" INTEGER NULL,
    "InviteMailEnabled" INTEGER NOT NULL,
    "RecoveryMailEnabled" INTEGER NOT NULL,
    "AlertMailEnabled" INTEGER NOT NULL,
    "BackupRetentionDays" INTEGER NOT NULL,
    "LogRetentionDays" INTEGER NOT NULL,
    "OrphanObjectRetentionDays" INTEGER NOT NULL,
    "AutomaticCleanupEnabled" INTEGER NOT NULL,
    "DesktopMinimumVersion" TEXT NULL,
    "DesktopRecommendedVersion" TEXT NULL,
    "MfaRequirement" TEXT NOT NULL DEFAULT 'optional',
    "TrustedDeviceDays" INTEGER NOT NULL DEFAULT 14,
    "AuthChallengeMinutes" INTEGER NOT NULL DEFAULT 5,
    "EmailCodeMinutes" INTEGER NOT NULL DEFAULT 8,
    "EmailCodeCooldownSeconds" INTEGER NOT NULL DEFAULT 45,
    "EmailCodeMaxAttempts" INTEGER NOT NULL DEFAULT 5,
    "EmailCodeSendsPerHour" INTEGER NOT NULL DEFAULT 10,
    "CliRecoveryMinutes" INTEGER NOT NULL DEFAULT 10,
    "UpdatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_OperationsSettings_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "Sections" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_Sections" PRIMARY KEY,
    "ServerId" TEXT NOT NULL,
    "Name" TEXT NOT NULL,
    "Position" INTEGER NOT NULL,
    CONSTRAINT "FK_Sections_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "ServerAdminSettings" (
    "ServerId" TEXT NOT NULL CONSTRAINT "PK_ServerAdminSettings" PRIMARY KEY,
    "AllMessages" INTEGER NOT NULL,
    "MentionsOnly" INTEGER NOT NULL,
    "VoiceJoinSound" INTEGER NOT NULL,
    "MutedChannelIdsJson" TEXT NOT NULL,
    "ServerMuted" INTEGER NOT NULL,
    CONSTRAINT "FK_ServerAdminSettings_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "ServerRoles" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_ServerRoles" PRIMARY KEY,
    "ServerId" TEXT NOT NULL,
    "Name" TEXT NOT NULL,
    "Color" TEXT NOT NULL,
    "Permissions" INTEGER NOT NULL,
    "Position" INTEGER NOT NULL,
    "IsSystem" INTEGER NOT NULL,
    "IsOwner" INTEGER NOT NULL,
    CONSTRAINT "FK_ServerRoles_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "AccountKeyBackups" (
    "UserId" TEXT NOT NULL CONSTRAINT "PK_AccountKeyBackups" PRIMARY KEY,
    "Salt" BLOB NOT NULL,
    "Iterations" INTEGER NOT NULL,
    "Verifier" BLOB NOT NULL,
    "Iv" BLOB NOT NULL,
    "Ciphertext" BLOB NOT NULL,
    "FailedAttempts" INTEGER NOT NULL,
    "UpdatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_AccountKeyBackups_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "AccountKeys" (
    "UserId" TEXT NOT NULL CONSTRAINT "PK_AccountKeys" PRIMARY KEY,
    "KeyId" TEXT NOT NULL,
    "Algorithm" TEXT NOT NULL,
    "PublicKey" BLOB NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_AccountKeys_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "AuthChallenges" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_AuthChallenges" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "Purpose" TEXT NOT NULL,
    "SecretHash" TEXT NOT NULL,
    "PrimarySource" TEXT NOT NULL,
    "ParentSessionId" TEXT NULL,
    "SecurityVersion" INTEGER NOT NULL,
    "ShortLived" INTEGER NOT NULL,
    "PendingIdentityId" TEXT NULL,
    "WebAuthnOptions" TEXT NULL,
    "Attempts" INTEGER NOT NULL,
    "MaxAttempts" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL,
    "ConsumedAt" TEXT NULL,
    CONSTRAINT "FK_AuthChallenges_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "ChannelPostTimes" (
    "ChannelId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "LastPostedAt" TEXT NOT NULL,
    CONSTRAINT "PK_ChannelPostTimes" PRIMARY KEY ("ChannelId", "UserId"),
    CONSTRAINT "FK_ChannelPostTimes_Channels_ChannelId" FOREIGN KEY ("ChannelId") REFERENCES "Channels" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ChannelPostTimes_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "ChannelReads" (
    "ChannelId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "LastReadAt" TEXT NOT NULL,
    CONSTRAINT "PK_ChannelReads" PRIMARY KEY ("ChannelId", "UserId"),
    CONSTRAINT "FK_ChannelReads_Channels_ChannelId" FOREIGN KEY ("ChannelId") REFERENCES "Channels" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ChannelReads_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "CliRecoveryCredentials" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_CliRecoveryCredentials" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "TokenHash" TEXT NOT NULL,
    "Reason" TEXT NOT NULL,
    "SecurityVersion" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL,
    "UsedAt" TEXT NULL,
    CONSTRAINT "FK_CliRecoveryCredentials_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "DesktopBrowserLogins" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_DesktopBrowserLogins" PRIMARY KEY,
    "SecretHash" TEXT NOT NULL,
    "ApprovalTokenHash" TEXT NULL,
    "ApprovalUserId" TEXT NULL,
    "ApprovalSessionId" TEXT NULL,
    "UserId" TEXT NULL,
    "ApprovedSessionId" TEXT NULL,
    "PrimarySource" TEXT NULL,
    "MfaCompletedAt" TEXT NULL,
    "MfaEvidence" TEXT NULL,
    "SecurityVersion" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL,
    "ApprovedAt" TEXT NULL,
    "RedeemedAt" TEXT NULL,
    CONSTRAINT "FK_DesktopBrowserLogins_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "DirectConversations" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_DirectConversations" PRIMARY KEY,
    "UserAId" TEXT NOT NULL,
    "UserBId" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "EncryptionState" TEXT NOT NULL,
    "EncryptionRequestedById" TEXT NULL,
    "EncryptionRequestedAt" TEXT NULL,
    "EncryptionEnabledAt" TEXT NULL,
    "CurrentEpochId" TEXT NULL,
    CONSTRAINT "FK_DirectConversations_Users_UserAId" FOREIGN KEY ("UserAId") REFERENCES "Users" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_DirectConversations_Users_UserBId" FOREIGN KEY ("UserBId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "DiscordImports" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_DiscordImports" PRIMARY KEY,
    "ServerId" TEXT NULL,
    "DiscordServerId" TEXT NOT NULL,
    "DiscordServerName" TEXT NOT NULL,
    "StartedById" TEXT NULL,
    "StartedAt" TEXT NOT NULL,
    CONSTRAINT "FK_DiscordImports_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE SET NULL,
    CONSTRAINT "FK_DiscordImports_Users_StartedById" FOREIGN KEY ("StartedById") REFERENCES "Users" ("Id") ON DELETE SET NULL
);
CREATE TABLE IF NOT EXISTS "EmailChangeRequests" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_EmailChangeRequests" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "SessionId" TEXT NULL,
    "SecurityVersion" INTEGER NULL,
    "NewEmail" TEXT NOT NULL,
    "NormalizedNewEmail" TEXT NOT NULL,
    "TokenHash" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL,
    "UsedAt" TEXT NULL,
    CONSTRAINT "FK_EmailChangeRequests_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "EmailOtpCodes" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_EmailOtpCodes" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "MethodId" TEXT NULL,
    "ChallengeId" TEXT NULL,
    "Purpose" TEXT NOT NULL,
    "EmailAddress" TEXT NOT NULL,
    "ProtectedCode" TEXT NOT NULL,
    "Attempts" INTEGER NOT NULL,
    "MaxAttempts" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL,
    "ConsumedAt" TEXT NULL,
    "DeliveredAt" TEXT NULL,
    CONSTRAINT "FK_EmailOtpCodes_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "ExternalIdentities" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_ExternalIdentities" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "Issuer" TEXT NOT NULL,
    "Subject" TEXT NOT NULL,
    "ProviderId" TEXT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_ExternalIdentities_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "FriendRequests" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_FriendRequests" PRIMARY KEY,
    "SenderId" TEXT NOT NULL,
    "ReceiverId" TEXT NOT NULL,
    "PairKey" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_FriendRequests_Users_ReceiverId" FOREIGN KEY ("ReceiverId") REFERENCES "Users" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_FriendRequests_Users_SenderId" FOREIGN KEY ("SenderId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "Friendships" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_Friendships" PRIMARY KEY,
    "UserAId" TEXT NOT NULL,
    "UserBId" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_Friendships_Users_UserAId" FOREIGN KEY ("UserAId") REFERENCES "Users" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_Friendships_Users_UserBId" FOREIGN KEY ("UserBId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "Messages" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_Messages" PRIMARY KEY,
    "ChannelId" TEXT NOT NULL,
    "AuthorId" TEXT NOT NULL,
    "IsSystem" INTEGER NOT NULL DEFAULT 0,
    "Content" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "EditedAt" TEXT NULL,
    "PinnedAt" TEXT NULL,
    "ReplyToMessageId" TEXT NULL,
    CONSTRAINT "FK_Messages_Messages_ReplyToMessageId" FOREIGN KEY ("ReplyToMessageId") REFERENCES "Messages" ("Id") ON DELETE SET NULL,
    CONSTRAINT "FK_Messages_Users_AuthorId" FOREIGN KEY ("AuthorId") REFERENCES "Users" ("Id") ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS "MfaMethods" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_MfaMethods" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "Type" TEXT NOT NULL,
    "Name" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ConfirmedAt" TEXT NULL,
    "LastUsedAt" TEXT NULL,
    "ProtectedSecret" TEXT NULL,
    "EmailAddress" TEXT NULL,
    "LastTotpStep" INTEGER NULL,
    "RevokedAt" TEXT NULL,
    CONSTRAINT "FK_MfaMethods_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "NotificationDeliveries" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_NotificationDeliveries" PRIMARY KEY,
    "SourceMessageId" TEXT NOT NULL,
    "RecipientId" TEXT NOT NULL,
    "EventKind" TEXT NOT NULL,
    "Channel" TEXT NOT NULL,
    "DestinationKey" TEXT NOT NULL,
    "PushSubscriptionId" TEXT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL,
    "Attempts" INTEGER NOT NULL,
    "NextAttemptAt" TEXT NOT NULL,
    "LastAttemptAt" TEXT NULL,
    "SentAt" TEXT NULL,
    "Status" TEXT NOT NULL,
    "LeaseId" TEXT NULL,
    "LeaseUntil" TEXT NULL,
    "LastError" TEXT NOT NULL DEFAULT '',
    CONSTRAINT "FK_NotificationDeliveries_Users_RecipientId" FOREIGN KEY ("RecipientId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "PasswordResetTokens" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_PasswordResetTokens" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "TokenHash" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL,
    "UsedAt" TEXT NULL,
    CONSTRAINT "FK_PasswordResetTokens_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "PushSubscriptions" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_PushSubscriptions" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "SessionId" TEXT NOT NULL,
    "Endpoint" TEXT NOT NULL,
    "P256dh" TEXT NOT NULL,
    "Auth" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_PushSubscriptions_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "RecoveryCodes" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_RecoveryCodes" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "BatchId" TEXT NOT NULL,
    "CodeHash" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "UsedAt" TEXT NULL,
    CONSTRAINT "FK_RecoveryCodes_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "SecurityEvents" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_SecurityEvents" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "Kind" TEXT NOT NULL,
    "Detail" TEXT NOT NULL,
    "IpAddress" TEXT NOT NULL,
    "Device" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_SecurityEvents_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "ServerAuditEntries" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_ServerAuditEntries" PRIMARY KEY,
    "ServerId" TEXT NOT NULL,
    "ActorId" TEXT NOT NULL,
    "Summary" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "Action" TEXT NULL,
    "TargetType" TEXT NULL,
    "TargetId" TEXT NULL,
    "TargetName" TEXT NULL,
    "ChangesJson" TEXT NULL,
    CONSTRAINT "FK_ServerAuditEntries_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ServerAuditEntries_Users_ActorId" FOREIGN KEY ("ActorId") REFERENCES "Users" ("Id") ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS "ServerBans" (
    "ServerId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "BannedById" TEXT NOT NULL,
    "Reason" TEXT NOT NULL,
    "BannedAt" TEXT NOT NULL,
    CONSTRAINT "PK_ServerBans" PRIMARY KEY ("ServerId", "UserId"),
    CONSTRAINT "FK_ServerBans_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ServerBans_Users_BannedById" FOREIGN KEY ("BannedById") REFERENCES "Users" ("Id") ON DELETE RESTRICT,
    CONSTRAINT "FK_ServerBans_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "ServerEmoji" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_ServerEmoji" PRIMARY KEY,
    "ServerId" TEXT NOT NULL,
    "Shortcode" TEXT NOT NULL,
    "ObjectKey" TEXT NOT NULL,
    "ContentType" TEXT NOT NULL,
    "SizeBytes" INTEGER NOT NULL,
    "UploadedById" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_ServerEmoji_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ServerEmoji_Users_UploadedById" FOREIGN KEY ("UploadedById") REFERENCES "Users" ("Id") ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS "ServerInvites" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_ServerInvites" PRIMARY KEY,
    "ServerId" TEXT NOT NULL,
    "CreatedById" TEXT NOT NULL,
    "ChannelId" TEXT NULL,
    "Code" TEXT NOT NULL,
    "Uses" INTEGER NOT NULL,
    "MaxUses" INTEGER NULL,
    "ExpiresAt" TEXT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_ServerInvites_Channels_ChannelId" FOREIGN KEY ("ChannelId") REFERENCES "Channels" ("Id") ON DELETE SET NULL,
    CONSTRAINT "FK_ServerInvites_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ServerInvites_Users_CreatedById" FOREIGN KEY ("CreatedById") REFERENCES "Users" ("Id") ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS "SoundboardSounds" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_SoundboardSounds" PRIMARY KEY,
    "ServerId" TEXT NOT NULL,
    "AttachmentId" TEXT NOT NULL,
    "CreatedById" TEXT NOT NULL,
    "Name" TEXT NOT NULL,
    "Glyph" TEXT NOT NULL,
    "TrimStartSeconds" REAL NOT NULL,
    "TrimEndSeconds" REAL NOT NULL,
    "Position" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_SoundboardSounds_ImageAttachments_AttachmentId" FOREIGN KEY ("AttachmentId") REFERENCES "ImageAttachments" ("Id") ON DELETE RESTRICT,
    CONSTRAINT "FK_SoundboardSounds_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_SoundboardSounds_Users_CreatedById" FOREIGN KEY ("CreatedById") REFERENCES "Users" ("Id") ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS "TrustedDevices" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_TrustedDevices" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "TokenHash" TEXT NOT NULL,
    "Name" TEXT NOT NULL,
    "Device" TEXT NOT NULL,
    "IpAddress" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL,
    "LastUsedAt" TEXT NULL,
    "RevokedAt" TEXT NULL,
    CONSTRAINT "FK_TrustedDevices_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "UserBlocks" (
    "BlockerId" TEXT NOT NULL,
    "BlockedId" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "PK_UserBlocks" PRIMARY KEY ("BlockerId", "BlockedId"),
    CONSTRAINT "FK_UserBlocks_Users_BlockedId" FOREIGN KEY ("BlockedId") REFERENCES "Users" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_UserBlocks_Users_BlockerId" FOREIGN KEY ("BlockerId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "UserNotifications" (
    "Sequence" INTEGER NOT NULL CONSTRAINT "PK_UserNotifications" PRIMARY KEY AUTOINCREMENT,
    "Id" TEXT NOT NULL,
    "ServerId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "Source" TEXT NOT NULL,
    "SourcePackageId" TEXT NOT NULL,
    "SourcePackageName" TEXT NOT NULL,
    "SourceModuleId" TEXT NOT NULL,
    "SourceModuleName" TEXT NOT NULL,
    "Title" TEXT NOT NULL,
    "Body" TEXT NOT NULL,
    "Tone" TEXT NOT NULL,
    "TargetChannelId" TEXT NULL,
    "TargetInviteCode" TEXT NULL,
    "CreatedAt" TEXT NOT NULL,
    "ReadAt" TEXT NULL,
    CONSTRAINT "FK_UserNotifications_Channels_TargetChannelId" FOREIGN KEY ("TargetChannelId") REFERENCES "Channels" ("Id") ON DELETE SET NULL,
    CONSTRAINT "FK_UserNotifications_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_UserNotifications_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "UserPreferences" (
    "UserId" TEXT NOT NULL CONSTRAINT "PK_UserPreferences" PRIMARY KEY,
    "DesktopNotifications" INTEGER NOT NULL,
    "MentionPopups" INTEGER NOT NULL,
    "CallRingtone" INTEGER NOT NULL,
    "EmailNotifications" INTEGER NOT NULL,
    "PushNotifications" INTEGER NOT NULL,
    "MentionsOnly" INTEGER NOT NULL,
    "MessageSound" INTEGER NOT NULL,
    "JoinLeaveSound" INTEGER NOT NULL,
    "AllowDirectMessages" INTEGER NOT NULL,
    "AllowFriendRequests" INTEGER NOT NULL,
    "ShowPresence" INTEGER NOT NULL,
    "DeveloperMode" INTEGER NOT NULL,
    "ReducedMotion" INTEGER NOT NULL,
    "MutedChannelIdsJson" TEXT NOT NULL DEFAULT '[]',
    "VadSensitivity" INTEGER NOT NULL,
    "JoinSoundId" TEXT NULL,
    "AppearanceSettingsJson" TEXT NULL,
    "LanguageSettingsJson" TEXT NULL,
    "KeybindSettingsJson" TEXT NULL,
    "VoiceSettingsJson" TEXT NULL,
    "PrivacySettingsJson" TEXT NULL,
    CONSTRAINT "FK_UserPreferences_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "UserSessions" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_UserSessions" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "Device" TEXT NOT NULL,
    "IpAddress" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "LastSeenAt" TEXT NOT NULL,
    "ExpiresAt" TEXT NOT NULL,
    "AbsoluteExpiresAt" TEXT NULL,
    "PrimarySource" TEXT NOT NULL DEFAULT 'password',
    "AuthenticatedAt" TEXT NULL,
    "MfaCompletedAt" TEXT NULL,
    "MfaEvidence" TEXT NULL,
    "SecurityVersion" INTEGER NOT NULL,
    "ShortLived" INTEGER NOT NULL,
    "RevokedAt" TEXT NULL,
    CONSTRAINT "FK_UserSessions_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "PermissionOverrides" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_PermissionOverrides" PRIMARY KEY,
    "ServerId" TEXT NOT NULL,
    "TargetType" INTEGER NOT NULL,
    "TargetId" TEXT NOT NULL,
    "RoleId" TEXT NOT NULL,
    "AllowMask" INTEGER NOT NULL,
    "DenyMask" INTEGER NOT NULL,
    CONSTRAINT "FK_PermissionOverrides_ServerRoles_RoleId" FOREIGN KEY ("RoleId") REFERENCES "ServerRoles" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_PermissionOverrides_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "ServerMembers" (
    "ServerId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "RoleId" TEXT NOT NULL,
    "OwnerSlot" INTEGER NULL,
    "JoinedAt" TEXT NOT NULL,
    CONSTRAINT "PK_ServerMembers" PRIMARY KEY ("ServerId", "UserId"),
    CONSTRAINT "FK_ServerMembers_ServerRoles_RoleId" FOREIGN KEY ("RoleId") REFERENCES "ServerRoles" ("Id") ON DELETE RESTRICT,
    CONSTRAINT "FK_ServerMembers_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_ServerMembers_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "DirectConversationReads" (
    "ConversationId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "LastReadAt" TEXT NOT NULL,
    CONSTRAINT "PK_DirectConversationReads" PRIMARY KEY ("ConversationId", "UserId"),
    CONSTRAINT "FK_DirectConversationReads_DirectConversations_ConversationId" FOREIGN KEY ("ConversationId") REFERENCES "DirectConversations" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_DirectConversationReads_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "EncryptionEpochs" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_EncryptionEpochs" PRIMARY KEY,
    "ConversationId" TEXT NOT NULL,
    "Sequence" INTEGER NOT NULL,
    "PreviousEpochId" TEXT NULL,
    "CreatedById" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_EncryptionEpochs_DirectConversations_ConversationId" FOREIGN KEY ("ConversationId") REFERENCES "DirectConversations" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_EncryptionEpochs_Users_CreatedById" FOREIGN KEY ("CreatedById") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "DiscordImportAccounts" (
    "DiscordId" TEXT NOT NULL CONSTRAINT "PK_DiscordImportAccounts" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "ImportId" TEXT NULL,
    "IsWebhook" INTEGER NOT NULL,
    "IsBot" INTEGER NOT NULL,
    "DiscordUsername" TEXT NOT NULL,
    "DiscordAvatarUrl" TEXT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_DiscordImportAccounts_DiscordImports_ImportId" FOREIGN KEY ("ImportId") REFERENCES "DiscordImports" ("Id") ON DELETE SET NULL,
    CONSTRAINT "FK_DiscordImportAccounts_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "DiscordImportChannels" (
    "ServerId" TEXT NOT NULL,
    "DiscordId" TEXT NOT NULL,
    "ImportId" TEXT NOT NULL,
    "Kind" INTEGER NOT NULL,
    "ChannelId" TEXT NULL,
    "DiscordType" INTEGER NOT NULL,
    "ParentDiscordId" TEXT NULL,
    "Name" TEXT NULL,
    "StarterMessageId" TEXT NULL,
    "IsPrivate" INTEGER NOT NULL,
    "OldestMessageId" TEXT NULL,
    "CompletedAt" TEXT NULL,
    "RootMessageId" TEXT NULL,
    "LinkedAt" TEXT NULL,
    CONSTRAINT "PK_DiscordImportChannels" PRIMARY KEY ("ServerId", "DiscordId"),
    CONSTRAINT "FK_DiscordImportChannels_Channels_ChannelId" FOREIGN KEY ("ChannelId") REFERENCES "Channels" ("Id") ON DELETE SET NULL,
    CONSTRAINT "FK_DiscordImportChannels_DiscordImports_ImportId" FOREIGN KEY ("ImportId") REFERENCES "DiscordImports" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_DiscordImportChannels_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "DiscordImportSections" (
    "ServerId" TEXT NOT NULL,
    "DiscordId" TEXT NOT NULL,
    "ImportId" TEXT NOT NULL,
    "SectionId" TEXT NULL,
    CONSTRAINT "PK_DiscordImportSections" PRIMARY KEY ("ServerId", "DiscordId"),
    CONSTRAINT "FK_DiscordImportSections_DiscordImports_ImportId" FOREIGN KEY ("ImportId") REFERENCES "DiscordImports" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_DiscordImportSections_Sections_SectionId" FOREIGN KEY ("SectionId") REFERENCES "Sections" ("Id") ON DELETE SET NULL,
    CONSTRAINT "FK_DiscordImportSections_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "DiscordImportMessages" (
    "ServerId" TEXT NOT NULL,
    "DiscordId" TEXT NOT NULL,
    "ImportId" TEXT NOT NULL,
    "ChannelDiscordId" TEXT NOT NULL,
    "Outcome" INTEGER NOT NULL,
    "Reason" INTEGER NOT NULL,
    "DiscordType" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "MessageId" TEXT NULL,
    "ReplyToDiscordId" TEXT NULL,
    "AttachmentsPending" INTEGER NOT NULL,
    CONSTRAINT "PK_DiscordImportMessages" PRIMARY KEY ("ServerId", "DiscordId"),
    CONSTRAINT "FK_DiscordImportMessages_DiscordImports_ImportId" FOREIGN KEY ("ImportId") REFERENCES "DiscordImports" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_DiscordImportMessages_Messages_MessageId" FOREIGN KEY ("MessageId") REFERENCES "Messages" ("Id") ON DELETE SET NULL,
    CONSTRAINT "FK_DiscordImportMessages_Servers_ServerId" FOREIGN KEY ("ServerId") REFERENCES "Servers" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "MessageAttachments" (
    "MessageId" TEXT NOT NULL,
    "AttachmentId" TEXT NOT NULL,
    CONSTRAINT "PK_MessageAttachments" PRIMARY KEY ("MessageId", "AttachmentId"),
    CONSTRAINT "FK_MessageAttachments_ImageAttachments_AttachmentId" FOREIGN KEY ("AttachmentId") REFERENCES "ImageAttachments" ("Id") ON DELETE RESTRICT,
    CONSTRAINT "FK_MessageAttachments_Messages_MessageId" FOREIGN KEY ("MessageId") REFERENCES "Messages" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "MessageEmbeds" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_MessageEmbeds" PRIMARY KEY,
    "MessageId" TEXT NOT NULL,
    "Url" TEXT NOT NULL,
    "Kind" INTEGER NOT NULL,
    "Provider" TEXT NOT NULL,
    "ProviderId" TEXT NULL,
    "StartSeconds" INTEGER NULL,
    "Domain" TEXT NOT NULL,
    "Title" TEXT NULL,
    "Description" TEXT NULL,
    "ThumbnailAttachmentId" TEXT NULL,
    "Position" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "FK_MessageEmbeds_ImageAttachments_ThumbnailAttachmentId" FOREIGN KEY ("ThumbnailAttachmentId") REFERENCES "ImageAttachments" ("Id") ON DELETE RESTRICT,
    CONSTRAINT "FK_MessageEmbeds_Messages_MessageId" FOREIGN KEY ("MessageId") REFERENCES "Messages" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "MessageIndexEntries" (
    "MessageId" TEXT NOT NULL CONSTRAINT "PK_MessageIndexEntries" PRIMARY KEY,
    "ChannelId" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "SearchText" TEXT NOT NULL DEFAULT '',
    "HasAttachments" INTEGER NOT NULL,
    "HasLinks" INTEGER NOT NULL,
    CONSTRAINT "FK_MessageIndexEntries_Messages_MessageId" FOREIGN KEY ("MessageId") REFERENCES "Messages" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "MessageReactions" (
    "MessageId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "Emoji" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    CONSTRAINT "PK_MessageReactions" PRIMARY KEY ("MessageId", "UserId", "Emoji"),
    CONSTRAINT "FK_MessageReactions_Messages_MessageId" FOREIGN KEY ("MessageId") REFERENCES "Messages" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_MessageReactions_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "WebAuthnCredentials" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_WebAuthnCredentials" PRIMARY KEY,
    "MethodId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "CredentialId" BLOB NOT NULL,
    "PublicKey" BLOB NOT NULL,
    "UserHandle" BLOB NOT NULL,
    "SignCount" INTEGER NOT NULL,
    "BackupEligible" INTEGER NOT NULL,
    "BackedUp" INTEGER NOT NULL,
    "AaGuid" TEXT NOT NULL,
    "Transports" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "LastUsedAt" TEXT NULL,
    CONSTRAINT "FK_WebAuthnCredentials_MfaMethods_MethodId" FOREIGN KEY ("MethodId") REFERENCES "MfaMethods" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_WebAuthnCredentials_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "SecurityNotifications" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_SecurityNotifications" PRIMARY KEY,
    "SecurityEventId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "EmailAddress" TEXT NOT NULL,
    "Attempts" INTEGER NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "NextAttemptAt" TEXT NOT NULL,
    "LastAttemptAt" TEXT NULL,
    "SentAt" TEXT NULL,
    "LastError" TEXT NOT NULL,
    CONSTRAINT "FK_SecurityNotifications_SecurityEvents_SecurityEventId" FOREIGN KEY ("SecurityEventId") REFERENCES "SecurityEvents" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_SecurityNotifications_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id")
);
CREATE TABLE IF NOT EXISTS "EncryptionDevices" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_EncryptionDevices" PRIMARY KEY,
    "UserId" TEXT NOT NULL,
    "SessionId" TEXT NOT NULL,
    "Algorithm" TEXT NOT NULL,
    "PublicKey" BLOB NOT NULL,
    "Label" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "LastSeenAt" TEXT NOT NULL,
    "RevokedAt" TEXT NULL,
    CONSTRAINT "FK_EncryptionDevices_UserSessions_SessionId" FOREIGN KEY ("SessionId") REFERENCES "UserSessions" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_EncryptionDevices_Users_UserId" FOREIGN KEY ("UserId") REFERENCES "Users" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "MemberPermissionOverrides" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_MemberPermissionOverrides" PRIMARY KEY,
    "ServerId" TEXT NOT NULL,
    "TargetType" INTEGER NOT NULL,
    "TargetId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "AllowMask" INTEGER NOT NULL,
    "DenyMask" INTEGER NOT NULL,
    CONSTRAINT "FK_MemberPermissionOverrides_ServerMembers_ServerId_UserId" FOREIGN KEY ("ServerId", "UserId") REFERENCES "ServerMembers" ("ServerId", "UserId") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "ServerMemberRoles" (
    "ServerId" TEXT NOT NULL,
    "UserId" TEXT NOT NULL,
    "RoleId" TEXT NOT NULL,
    CONSTRAINT "PK_ServerMemberRoles" PRIMARY KEY ("ServerId", "UserId", "RoleId"),
    CONSTRAINT "FK_ServerMemberRoles_ServerMembers_ServerId_UserId" FOREIGN KEY ("ServerId", "UserId") REFERENCES "ServerMembers" ("ServerId", "UserId") ON DELETE CASCADE,
    CONSTRAINT "FK_ServerMemberRoles_ServerRoles_RoleId" FOREIGN KEY ("RoleId") REFERENCES "ServerRoles" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "DirectMessages" (
    "Id" TEXT NOT NULL CONSTRAINT "PK_DirectMessages" PRIMARY KEY,
    "ConversationId" TEXT NOT NULL,
    "AuthorId" TEXT NOT NULL,
    "Content" TEXT NOT NULL,
    "CreatedAt" TEXT NOT NULL,
    "EncryptedEpochId" TEXT NULL,
    "EncryptedIv" BLOB NULL,
    "EncryptedCiphertext" BLOB NULL,
    "SystemKind" TEXT NULL,
    CONSTRAINT "FK_DirectMessages_DirectConversations_ConversationId" FOREIGN KEY ("ConversationId") REFERENCES "DirectConversations" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_DirectMessages_EncryptionEpochs_EncryptedEpochId" FOREIGN KEY ("EncryptedEpochId") REFERENCES "EncryptionEpochs" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_DirectMessages_Users_AuthorId" FOREIGN KEY ("AuthorId") REFERENCES "Users" ("Id") ON DELETE RESTRICT
);
CREATE TABLE IF NOT EXISTS "EncryptionEnvelopes" (
    "EpochId" TEXT NOT NULL,
    "RecipientKeyId" TEXT NOT NULL,
    "RecipientUserId" TEXT NOT NULL,
    "RecipientKind" TEXT NOT NULL,
    "EphemeralPublicKey" BLOB NOT NULL,
    "Iv" BLOB NOT NULL,
    "Ciphertext" BLOB NOT NULL,
    CONSTRAINT "PK_EncryptionEnvelopes" PRIMARY KEY ("EpochId", "RecipientKeyId"),
    CONSTRAINT "FK_EncryptionEnvelopes_EncryptionEpochs_EpochId" FOREIGN KEY ("EpochId") REFERENCES "EncryptionEpochs" ("Id") ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS "DirectMessageAttachments" (
    "DirectMessageId" TEXT NOT NULL,
    "AttachmentId" TEXT NOT NULL,
    CONSTRAINT "PK_DirectMessageAttachments" PRIMARY KEY ("DirectMessageId", "AttachmentId"),
    CONSTRAINT "FK_DirectMessageAttachments_DirectMessages_DirectMessageId" FOREIGN KEY ("DirectMessageId") REFERENCES "DirectMessages" ("Id") ON DELETE CASCADE,
    CONSTRAINT "FK_DirectMessageAttachments_ImageAttachments_AttachmentId" FOREIGN KEY ("AttachmentId") REFERENCES "ImageAttachments" ("Id") ON DELETE RESTRICT
);
CREATE UNIQUE INDEX "IX_AccountKeys_KeyId" ON "AccountKeys" ("KeyId");
CREATE INDEX "IX_AuthChallenges_UserId_Purpose_ExpiresAt" ON "AuthChallenges" ("UserId", "Purpose", "ExpiresAt");
CREATE INDEX "IX_ChannelPostTimes_UserId" ON "ChannelPostTimes" ("UserId");
CREATE INDEX "IX_ChannelReads_UserId" ON "ChannelReads" ("UserId");
CREATE INDEX "IX_Channels_SectionId" ON "Channels" ("SectionId");
CREATE UNIQUE INDEX "IX_Channels_ServerId_Name" ON "Channels" ("ServerId", "Name");
CREATE UNIQUE INDEX "IX_CliRecoveryCredentials_TokenHash" ON "CliRecoveryCredentials" ("TokenHash");
CREATE INDEX "IX_CliRecoveryCredentials_UserId_ExpiresAt" ON "CliRecoveryCredentials" ("UserId", "ExpiresAt");
CREATE INDEX "IX_DesktopBrowserLogins_ExpiresAt" ON "DesktopBrowserLogins" ("ExpiresAt");
CREATE INDEX "IX_DesktopBrowserLogins_UserId" ON "DesktopBrowserLogins" ("UserId");
CREATE INDEX "IX_DirectConversationReads_UserId" ON "DirectConversationReads" ("UserId");
CREATE UNIQUE INDEX "IX_DirectConversations_UserAId_UserBId" ON "DirectConversations" ("UserAId", "UserBId");
CREATE INDEX "IX_DirectConversations_UserBId" ON "DirectConversations" ("UserBId");
CREATE INDEX "IX_DirectMessageAttachments_AttachmentId" ON "DirectMessageAttachments" ("AttachmentId");
CREATE INDEX "IX_DirectMessages_AuthorId" ON "DirectMessages" ("AuthorId");
CREATE INDEX "IX_DirectMessages_ConversationId_CreatedAt" ON "DirectMessages" ("ConversationId", "CreatedAt");
CREATE INDEX "IX_DirectMessages_EncryptedEpochId" ON "DirectMessages" ("EncryptedEpochId");
CREATE INDEX "IX_DiscordImportAccounts_ImportId" ON "DiscordImportAccounts" ("ImportId");
CREATE UNIQUE INDEX "IX_DiscordImportAccounts_UserId" ON "DiscordImportAccounts" ("UserId");
CREATE INDEX "IX_DiscordImportChannels_ChannelId" ON "DiscordImportChannels" ("ChannelId");
CREATE INDEX "IX_DiscordImportChannels_ImportId" ON "DiscordImportChannels" ("ImportId");
CREATE INDEX "IX_DiscordImportChannels_ServerId_ParentDiscordId" ON "DiscordImportChannels" ("ServerId", "ParentDiscordId");
CREATE INDEX "IX_DiscordImportMessages_ImportId" ON "DiscordImportMessages" ("ImportId");
CREATE INDEX "IX_DiscordImportMessages_MessageId" ON "DiscordImportMessages" ("MessageId");
CREATE INDEX "IX_DiscordImportMessages_ServerId_ChannelDiscordId_Outcome" ON "DiscordImportMessages" ("ServerId", "ChannelDiscordId", "Outcome");
CREATE INDEX "IX_DiscordImports_ServerId" ON "DiscordImports" ("ServerId");
CREATE INDEX "IX_DiscordImports_StartedById" ON "DiscordImports" ("StartedById");
CREATE INDEX "IX_DiscordImportSections_ImportId" ON "DiscordImportSections" ("ImportId");
CREATE INDEX "IX_DiscordImportSections_SectionId" ON "DiscordImportSections" ("SectionId");
CREATE UNIQUE INDEX "IX_EmailChangeRequests_TokenHash" ON "EmailChangeRequests" ("TokenHash");
CREATE INDEX "IX_EmailChangeRequests_UserId_ExpiresAt" ON "EmailChangeRequests" ("UserId", "ExpiresAt");
CREATE INDEX "IX_EmailOtpCodes_ChallengeId_Purpose" ON "EmailOtpCodes" ("ChallengeId", "Purpose");
CREATE INDEX "IX_EmailOtpCodes_UserId_Purpose_CreatedAt" ON "EmailOtpCodes" ("UserId", "Purpose", "CreatedAt");
CREATE UNIQUE INDEX "IX_EncryptionDevices_SessionId" ON "EncryptionDevices" ("SessionId") WHERE "RevokedAt" IS NULL;
CREATE INDEX "IX_EncryptionDevices_UserId" ON "EncryptionDevices" ("UserId");
CREATE UNIQUE INDEX "IX_EncryptionEpochs_ConversationId_Sequence" ON "EncryptionEpochs" ("ConversationId", "Sequence");
CREATE INDEX "IX_EncryptionEpochs_CreatedById" ON "EncryptionEpochs" ("CreatedById");
CREATE UNIQUE INDEX "IX_ExternalIdentities_Issuer_Subject" ON "ExternalIdentities" ("Issuer", "Subject");
CREATE UNIQUE INDEX "IX_ExternalIdentities_UserId_Issuer" ON "ExternalIdentities" ("UserId", "Issuer");
CREATE UNIQUE INDEX "IX_FriendRequests_PairKey" ON "FriendRequests" ("PairKey");
CREATE INDEX "IX_FriendRequests_ReceiverId" ON "FriendRequests" ("ReceiverId");
CREATE INDEX "IX_FriendRequests_SenderId" ON "FriendRequests" ("SenderId");
CREATE UNIQUE INDEX "IX_Friendships_UserAId_UserBId" ON "Friendships" ("UserAId", "UserBId");
CREATE INDEX "IX_Friendships_UserBId" ON "Friendships" ("UserBId");
CREATE UNIQUE INDEX "IX_ImageAttachments_ObjectKey" ON "ImageAttachments" ("ObjectKey");
CREATE INDEX "IX_MemberPermissionOverrides_ServerId_UserId" ON "MemberPermissionOverrides" ("ServerId", "UserId");
CREATE UNIQUE INDEX "IX_MemberPermissionOverrides_TargetType_TargetId_UserId" ON "MemberPermissionOverrides" ("TargetType", "TargetId", "UserId");
CREATE INDEX "IX_MessageAttachments_AttachmentId" ON "MessageAttachments" ("AttachmentId");
CREATE INDEX "IX_MessageChanges_ChannelId_Sequence" ON "MessageChanges" ("ChannelId", "Sequence");
CREATE INDEX "IX_MessageEmbeds_MessageId_Position" ON "MessageEmbeds" ("MessageId", "Position");
CREATE INDEX "IX_MessageEmbeds_ThumbnailAttachmentId" ON "MessageEmbeds" ("ThumbnailAttachmentId");
CREATE INDEX "IX_MessageIndexEntries_ChannelId_HasAttachments_CreatedAt_MessageId" ON "MessageIndexEntries" ("ChannelId", "HasAttachments", "CreatedAt" DESC, "MessageId" DESC);
CREATE INDEX "IX_MessageIndexEntries_ChannelId_HasLinks_CreatedAt_MessageId" ON "MessageIndexEntries" ("ChannelId", "HasLinks", "CreatedAt" DESC, "MessageId" DESC);
CREATE INDEX "IX_MessageReactions_MessageId_Emoji" ON "MessageReactions" ("MessageId", "Emoji");
CREATE INDEX "IX_MessageReactions_UserId" ON "MessageReactions" ("UserId");
CREATE INDEX "IX_Messages_AuthorId" ON "Messages" ("AuthorId");
CREATE INDEX "IX_Messages_ChannelId_CreatedAt" ON "Messages" ("ChannelId", "CreatedAt");
CREATE INDEX "IX_Messages_ChannelId_PinnedAt" ON "Messages" ("ChannelId", "PinnedAt");
CREATE INDEX "IX_Messages_ReplyToMessageId" ON "Messages" ("ReplyToMessageId");
CREATE INDEX "IX_MfaMethods_UserId_RevokedAt" ON "MfaMethods" ("UserId", "RevokedAt");
CREATE INDEX "IX_NotificationDeliveries_ExpiresAt" ON "NotificationDeliveries" ("ExpiresAt");
CREATE INDEX "IX_NotificationDeliveries_PushSubscriptionId" ON "NotificationDeliveries" ("PushSubscriptionId");
CREATE INDEX "IX_NotificationDeliveries_RecipientId_Channel" ON "NotificationDeliveries" ("RecipientId", "Channel");
CREATE INDEX "IX_NotificationDeliveries_Status_NextAttemptAt_LeaseUntil" ON "NotificationDeliveries" ("Status", "NextAttemptAt", "LeaseUntil");
CREATE UNIQUE INDEX "IX_NotificationDeliveries_Unique" ON "NotificationDeliveries" ("SourceMessageId", "RecipientId", "EventKind", "Channel", "DestinationKey");
CREATE INDEX "IX_ObjectDeletions_QueuedAt" ON "ObjectDeletions" ("QueuedAt");
CREATE INDEX "IX_OperatorAuditEntries_CreatedAt" ON "OperatorAuditEntries" ("CreatedAt");
CREATE INDEX "IX_PasskeySignIns_ExpiresAt" ON "PasskeySignIns" ("ExpiresAt");
CREATE UNIQUE INDEX "IX_PasswordResetTokens_TokenHash" ON "PasswordResetTokens" ("TokenHash");
CREATE INDEX "IX_PasswordResetTokens_UserId_ExpiresAt" ON "PasswordResetTokens" ("UserId", "ExpiresAt");
CREATE INDEX "IX_PendingExternalIdentities_ExpiresAt" ON "PendingExternalIdentities" ("ExpiresAt");
CREATE INDEX "IX_PermissionOverrides_RoleId" ON "PermissionOverrides" ("RoleId");
CREATE INDEX "IX_PermissionOverrides_ServerId" ON "PermissionOverrides" ("ServerId");
CREATE UNIQUE INDEX "IX_PermissionOverrides_TargetType_TargetId_RoleId" ON "PermissionOverrides" ("TargetType", "TargetId", "RoleId");
CREATE UNIQUE INDEX "IX_PushSubscriptions_Endpoint" ON "PushSubscriptions" ("Endpoint");
CREATE INDEX "IX_PushSubscriptions_UserId_SessionId" ON "PushSubscriptions" ("UserId", "SessionId");
CREATE UNIQUE INDEX "IX_RecoveryCodes_CodeHash" ON "RecoveryCodes" ("CodeHash");
CREATE INDEX "IX_RecoveryCodes_UserId_BatchId" ON "RecoveryCodes" ("UserId", "BatchId");
CREATE INDEX "IX_Sections_ServerId_Position" ON "Sections" ("ServerId", "Position");
CREATE INDEX "IX_SecurityEvents_UserId_CreatedAt" ON "SecurityEvents" ("UserId", "CreatedAt");
CREATE INDEX "IX_SecurityNotifications_SecurityEventId" ON "SecurityNotifications" ("SecurityEventId");
CREATE INDEX "IX_SecurityNotifications_SentAt_NextAttemptAt" ON "SecurityNotifications" ("SentAt", "NextAttemptAt");
CREATE INDEX "IX_SecurityNotifications_UserId" ON "SecurityNotifications" ("UserId");
CREATE INDEX "IX_ServerAuditEntries_ActorId" ON "ServerAuditEntries" ("ActorId");
CREATE INDEX "IX_ServerAuditEntries_ServerId_CreatedAt" ON "ServerAuditEntries" ("ServerId", "CreatedAt");
CREATE INDEX "IX_ServerBans_BannedById" ON "ServerBans" ("BannedById");
CREATE INDEX "IX_ServerBans_ServerId_BannedAt" ON "ServerBans" ("ServerId", "BannedAt");
CREATE INDEX "IX_ServerBans_UserId" ON "ServerBans" ("UserId");
CREATE UNIQUE INDEX "IX_ServerEmoji_ObjectKey" ON "ServerEmoji" ("ObjectKey");
CREATE UNIQUE INDEX "IX_ServerEmoji_ServerId_Shortcode" ON "ServerEmoji" ("ServerId", "Shortcode");
CREATE INDEX "IX_ServerEmoji_UploadedById" ON "ServerEmoji" ("UploadedById");
CREATE INDEX "IX_ServerInvites_ChannelId" ON "ServerInvites" ("ChannelId");
CREATE UNIQUE INDEX "IX_ServerInvites_Code" ON "ServerInvites" ("Code");
CREATE INDEX "IX_ServerInvites_CreatedById" ON "ServerInvites" ("CreatedById");
CREATE INDEX "IX_ServerInvites_ServerId" ON "ServerInvites" ("ServerId");
CREATE INDEX "IX_ServerMemberRoles_RoleId" ON "ServerMemberRoles" ("RoleId");
CREATE INDEX "IX_ServerMembers_RoleId" ON "ServerMembers" ("RoleId");
CREATE UNIQUE INDEX "IX_ServerMembers_ServerId_OwnerSlot" ON "ServerMembers" ("ServerId", "OwnerSlot");
CREATE INDEX "IX_ServerMembers_UserId" ON "ServerMembers" ("UserId");
CREATE UNIQUE INDEX "IX_ServerRoles_ServerId_Name" ON "ServerRoles" ("ServerId", "Name");
CREATE UNIQUE INDEX "IX_SoundboardSounds_AttachmentId" ON "SoundboardSounds" ("AttachmentId");
CREATE INDEX "IX_SoundboardSounds_CreatedById" ON "SoundboardSounds" ("CreatedById");
CREATE INDEX "IX_SoundboardSounds_ServerId_Position" ON "SoundboardSounds" ("ServerId", "Position");
CREATE UNIQUE INDEX "IX_TrustedDevices_TokenHash" ON "TrustedDevices" ("TokenHash");
CREATE INDEX "IX_TrustedDevices_UserId_ExpiresAt" ON "TrustedDevices" ("UserId", "ExpiresAt");
CREATE INDEX "IX_UserBlocks_BlockedId" ON "UserBlocks" ("BlockedId");
CREATE UNIQUE INDEX "IX_UserNotifications_Id" ON "UserNotifications" ("Id");
CREATE INDEX "IX_UserNotifications_ServerId_UserId_ReadAt_Sequence" ON "UserNotifications" ("ServerId", "UserId", "ReadAt", "Sequence");
CREATE INDEX "IX_UserNotifications_ServerId_UserId_Sequence" ON "UserNotifications" ("ServerId", "UserId", "Sequence");
CREATE INDEX "IX_UserNotifications_TargetChannelId" ON "UserNotifications" ("TargetChannelId");
CREATE INDEX "IX_UserNotifications_UserId_ReadAt_Sequence" ON "UserNotifications" ("UserId", "ReadAt", "Sequence");
CREATE UNIQUE INDEX "IX_Users_NormalizedEmail" ON "Users" ("NormalizedEmail");
CREATE UNIQUE INDEX "IX_Users_NormalizedUsername" ON "Users" ("NormalizedUsername");
CREATE INDEX "IX_UserSessions_UserId_LastSeenAt" ON "UserSessions" ("UserId", "LastSeenAt");
CREATE UNIQUE INDEX "IX_WebAuthnCredentials_CredentialId" ON "WebAuthnCredentials" ("CredentialId");
CREATE INDEX "IX_WebAuthnCredentials_MethodId" ON "WebAuthnCredentials" ("MethodId");
CREATE INDEX "IX_WebAuthnCredentials_UserId" ON "WebAuthnCredentials" ("UserId");
CREATE TRIGGER "Messages_PostTime_AfterInsert"
AFTER INSERT ON "Messages" WHEN new."IsSystem" = 0
BEGIN
    INSERT INTO "ChannelPostTimes" ("ChannelId", "UserId", "LastPostedAt")
    VALUES (new."ChannelId", new."AuthorId", new."CreatedAt")
    ON CONFLICT ("ChannelId", "UserId") DO UPDATE
    SET "LastPostedAt" = max("LastPostedAt", excluded."LastPostedAt");
END;
CREATE TRIGGER "Messages_Changes_AfterInsert"
AFTER INSERT ON "Messages"
BEGIN
    INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
    VALUES (new."ChannelId", new."Id", 0, strftime('%Y-%m-%d %H:%M:%f', 'now'));
END;
CREATE TRIGGER "Messages_Changes_AfterUpdate"
AFTER UPDATE OF "Content", "EditedAt", "PinnedAt" ON "Messages"
BEGIN
    INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
    VALUES (new."ChannelId", new."Id", 0, strftime('%Y-%m-%d %H:%M:%f', 'now'));
END;
CREATE TRIGGER "Messages_Changes_AfterDelete"
AFTER DELETE ON "Messages"
BEGIN
    INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
    VALUES (old."ChannelId", old."Id", 1, strftime('%Y-%m-%d %H:%M:%f', 'now'));
END;
CREATE TRIGGER "Channels_Changes_AfterDelete"
AFTER DELETE ON "Channels"
BEGIN
    DELETE FROM "MessageChanges" WHERE "ChannelId" = old."Id";
END;
CREATE TRIGGER "MessageReactions_Changes_AfterInsert"
AFTER INSERT ON "MessageReactions"
BEGIN
    INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
    SELECT "ChannelId", new."MessageId", 0, strftime('%Y-%m-%d %H:%M:%f', 'now')
    FROM "Messages" WHERE "Id" = new."MessageId";
END;
CREATE TRIGGER "MessageReactions_Changes_AfterDelete"
AFTER DELETE ON "MessageReactions"
BEGIN
    INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
    SELECT "ChannelId", old."MessageId", 0, strftime('%Y-%m-%d %H:%M:%f', 'now')
    FROM "Messages" WHERE "Id" = old."MessageId";
END;
CREATE TRIGGER "MessageEmbeds_Changes_AfterInsert"
AFTER INSERT ON "MessageEmbeds"
BEGIN
    INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
    SELECT "ChannelId", new."MessageId", 0, strftime('%Y-%m-%d %H:%M:%f', 'now')
    FROM "Messages" WHERE "Id" = new."MessageId";
END;
CREATE TRIGGER "MessageEmbeds_Changes_AfterDelete"
AFTER DELETE ON "MessageEmbeds"
BEGIN
    INSERT INTO "MessageChanges" ("ChannelId", "MessageId", "Kind", "CreatedAt")
    SELECT "ChannelId", old."MessageId", 0, strftime('%Y-%m-%d %H:%M:%f', 'now')
    FROM "Messages" WHERE "Id" = old."MessageId";
END;
CREATE VIRTUAL TABLE "MessageSearch" USING fts5(
    "MessageId" UNINDEXED,
    "ChannelId" UNINDEXED,
    "SearchText",
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3 4'
);
CREATE TABLE IF NOT EXISTS 'MessageSearch_data'(id INTEGER PRIMARY KEY, block BLOB);
CREATE TABLE IF NOT EXISTS 'MessageSearch_idx'(segid, term, pgno, PRIMARY KEY(segid, term)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS 'MessageSearch_content'(id INTEGER PRIMARY KEY, c0, c1, c2);
CREATE TABLE IF NOT EXISTS 'MessageSearch_docsize'(id INTEGER PRIMARY KEY, sz BLOB);
CREATE TABLE IF NOT EXISTS 'MessageSearch_config'(k PRIMARY KEY, v) WITHOUT ROWID;
CREATE TRIGGER "MessageIndexEntries_Search_AfterInsert"
AFTER INSERT ON "MessageIndexEntries"
BEGIN
    INSERT INTO "MessageSearch" (rowid, "MessageId", "ChannelId", "SearchText")
    VALUES (new.rowid, new."MessageId", new."ChannelId", new."SearchText");
END;
CREATE TRIGGER "MessageIndexEntries_Search_AfterUpdate"
AFTER UPDATE OF "MessageId", "ChannelId", "SearchText" ON "MessageIndexEntries"
BEGIN
    UPDATE "MessageSearch"
    SET "MessageId" = new."MessageId",
        "ChannelId" = new."ChannelId",
        "SearchText" = new."SearchText"
    WHERE rowid = old.rowid;
END;
CREATE TRIGGER "MessageIndexEntries_Search_AfterDelete"
AFTER DELETE ON "MessageIndexEntries"
BEGIN
    DELETE FROM "MessageSearch" WHERE rowid = old.rowid;
END;
CREATE TRIGGER "Messages_Index_AfterInsert"
AFTER INSERT ON "Messages"
BEGIN
    INSERT INTO "MessageIndexEntries" (
        "MessageId", "ChannelId", "CreatedAt", "SearchText",
        "HasAttachments", "HasLinks"
    )
    SELECT
        new."Id",
        new."ChannelId",
        new."CreatedAt",
        trim(
            coalesce(new."Content", '') || ' ' ||
            coalesce((
                SELECT u."DisplayName" || ' ' || u."Username"
                FROM "Users" u WHERE u."Id" = new."AuthorId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                FROM "MessageAttachments" ma
                JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                WHERE ma."MessageId" = new."Id"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(
                    e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                    coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                    ' '
                )
                FROM "MessageEmbeds" e WHERE e."MessageId" = new."Id"
            ), '')
        ),
        EXISTS(SELECT 1 FROM "MessageAttachments" ma WHERE ma."MessageId" = new."Id"),
        CASE WHEN
            instr(lower(coalesce(new."Content", '')), 'https://') > 0 OR
            instr(lower(coalesce(new."Content", '')), 'http://') > 0 OR
            EXISTS(SELECT 1 FROM "MessageEmbeds" e WHERE e."MessageId" = new."Id")
        THEN 1 ELSE 0 END;
END;
CREATE TRIGGER "Messages_Index_AfterUpdate"
AFTER UPDATE OF "ChannelId", "AuthorId", "Content", "CreatedAt" ON "Messages"
BEGIN
    UPDATE "MessageIndexEntries"
    SET "ChannelId" = new."ChannelId",
        "CreatedAt" = new."CreatedAt",
        "SearchText" = trim(
            coalesce(new."Content", '') || ' ' ||
            coalesce((
                SELECT u."DisplayName" || ' ' || u."Username"
                FROM "Users" u WHERE u."Id" = new."AuthorId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                FROM "MessageAttachments" ma
                JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                WHERE ma."MessageId" = new."Id"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(
                    e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                    coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                    ' '
                )
                FROM "MessageEmbeds" e WHERE e."MessageId" = new."Id"
            ), '')
        ),
        "HasLinks" = CASE WHEN
            instr(lower(coalesce(new."Content", '')), 'https://') > 0 OR
            instr(lower(coalesce(new."Content", '')), 'http://') > 0 OR
            EXISTS(SELECT 1 FROM "MessageEmbeds" e WHERE e."MessageId" = new."Id")
        THEN 1 ELSE 0 END
    WHERE "MessageId" = new."Id";
END;
CREATE TRIGGER "MessageAttachments_Index_AfterInsert"
AFTER INSERT ON "MessageAttachments"
BEGIN
    UPDATE "MessageIndexEntries"
    SET "SearchText" = trim(
            coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = new."MessageId"), '') || ' ' ||
            coalesce((
                SELECT u."DisplayName" || ' ' || u."Username"
                FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                WHERE m."Id" = new."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                FROM "MessageAttachments" ma
                JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                WHERE ma."MessageId" = new."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(
                    e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                    coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                    ' '
                )
                FROM "MessageEmbeds" e WHERE e."MessageId" = new."MessageId"
            ), '')
        ),
        "HasAttachments" = 1
    WHERE "MessageId" = new."MessageId";
END;
CREATE TRIGGER "MessageAttachments_Index_AfterDelete"
AFTER DELETE ON "MessageAttachments"
BEGIN
    UPDATE "MessageIndexEntries"
    SET "SearchText" = trim(
            coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = old."MessageId"), '') || ' ' ||
            coalesce((
                SELECT u."DisplayName" || ' ' || u."Username"
                FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                WHERE m."Id" = old."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                FROM "MessageAttachments" ma
                JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                WHERE ma."MessageId" = old."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(
                    e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                    coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                    ' '
                )
                FROM "MessageEmbeds" e WHERE e."MessageId" = old."MessageId"
            ), '')
        ),
        "HasAttachments" = EXISTS(
            SELECT 1 FROM "MessageAttachments" ma WHERE ma."MessageId" = old."MessageId"
        )
    WHERE "MessageId" = old."MessageId";
END;
CREATE TRIGGER "MessageEmbeds_Index_AfterInsert"
AFTER INSERT ON "MessageEmbeds"
BEGIN
    UPDATE "MessageIndexEntries"
    SET "SearchText" = trim(
            coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = new."MessageId"), '') || ' ' ||
            coalesce((
                SELECT u."DisplayName" || ' ' || u."Username"
                FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                WHERE m."Id" = new."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                FROM "MessageAttachments" ma
                JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                WHERE ma."MessageId" = new."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(
                    e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                    coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                    ' '
                )
                FROM "MessageEmbeds" e WHERE e."MessageId" = new."MessageId"
            ), '')
        ),
        "HasLinks" = 1
    WHERE "MessageId" = new."MessageId";
END;
CREATE TRIGGER "MessageEmbeds_Index_AfterUpdate"
AFTER UPDATE OF "Url", "Domain", "Provider", "Title", "Description", "MessageId" ON "MessageEmbeds"
BEGIN
    UPDATE "MessageIndexEntries"
    SET "SearchText" = trim(
            coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = new."MessageId"), '') || ' ' ||
            coalesce((
                SELECT u."DisplayName" || ' ' || u."Username"
                FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                WHERE m."Id" = new."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                FROM "MessageAttachments" ma
                JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                WHERE ma."MessageId" = new."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(
                    e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                    coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                    ' '
                )
                FROM "MessageEmbeds" e WHERE e."MessageId" = new."MessageId"
            ), '')
        ),
        "HasLinks" = 1
    WHERE "MessageId" = new."MessageId";
END;
CREATE TRIGGER "MessageEmbeds_Index_AfterDelete"
AFTER DELETE ON "MessageEmbeds"
BEGIN
    UPDATE "MessageIndexEntries"
    SET "SearchText" = trim(
            coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = old."MessageId"), '') || ' ' ||
            coalesce((
                SELECT u."DisplayName" || ' ' || u."Username"
                FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                WHERE m."Id" = old."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                FROM "MessageAttachments" ma
                JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                WHERE ma."MessageId" = old."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(
                    e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                    coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                    ' '
                )
                FROM "MessageEmbeds" e WHERE e."MessageId" = old."MessageId"
            ), '')
        ),
        "HasLinks" = CASE WHEN
            instr(lower(coalesce((
                SELECT m."Content" FROM "Messages" m WHERE m."Id" = old."MessageId"
            ), '')), 'https://') > 0 OR
            instr(lower(coalesce((
                SELECT m."Content" FROM "Messages" m WHERE m."Id" = old."MessageId"
            ), '')), 'http://') > 0 OR
            EXISTS(SELECT 1 FROM "MessageEmbeds" e WHERE e."MessageId" = old."MessageId")
        THEN 1 ELSE 0 END
    WHERE "MessageId" = old."MessageId";
END;
CREATE TRIGGER "ImageAttachments_Index_AfterUpdate"
AFTER UPDATE OF "FileName", "ContentType" ON "ImageAttachments"
BEGIN
    UPDATE "MessageIndexEntries"
    SET "SearchText" = trim(
            coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = "MessageIndexEntries"."MessageId"), '') || ' ' ||
            coalesce((
                SELECT u."DisplayName" || ' ' || u."Username"
                FROM "Messages" m JOIN "Users" u ON u."Id" = m."AuthorId"
                WHERE m."Id" = "MessageIndexEntries"."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                FROM "MessageAttachments" ma
                JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                WHERE ma."MessageId" = "MessageIndexEntries"."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(
                    e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                    coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                    ' '
                )
                FROM "MessageEmbeds" e WHERE e."MessageId" = "MessageIndexEntries"."MessageId"
            ), '')
        )
    WHERE "MessageId" IN (
        SELECT ma."MessageId" FROM "MessageAttachments" ma
        WHERE ma."AttachmentId" = new."Id"
    );
END;
CREATE TRIGGER "Users_MessageIndex_AfterUpdate"
AFTER UPDATE OF "DisplayName", "Username" ON "Users"
BEGIN
    UPDATE "MessageIndexEntries"
    SET "SearchText" = trim(
            coalesce((SELECT m."Content" FROM "Messages" m WHERE m."Id" = "MessageIndexEntries"."MessageId"), '') || ' ' ||
            new."DisplayName" || ' ' || new."Username" || ' ' ||
            coalesce((
                SELECT group_concat(a."FileName" || ' ' || a."ContentType", ' ')
                FROM "MessageAttachments" ma
                JOIN "ImageAttachments" a ON a."Id" = ma."AttachmentId"
                WHERE ma."MessageId" = "MessageIndexEntries"."MessageId"
            ), '') || ' ' ||
            coalesce((
                SELECT group_concat(
                    e."Url" || ' ' || e."Domain" || ' ' || e."Provider" || ' ' ||
                    coalesce(e."Title", '') || ' ' || coalesce(e."Description", ''),
                    ' '
                )
                FROM "MessageEmbeds" e WHERE e."MessageId" = "MessageIndexEntries"."MessageId"
            ), '')
        )
    WHERE "MessageId" IN (
        SELECT m."Id" FROM "Messages" m WHERE m."AuthorId" = new."Id"
    );
END;
CREATE TRIGGER "Servers_Channels_AfterDelete"
AFTER DELETE ON "Servers"
BEGIN
    DELETE FROM "Channels" WHERE "ServerId" = old."Id";
END;
CREATE TRIGGER "Channels_Messages_AfterDelete"
AFTER DELETE ON "Channels"
BEGIN
    DELETE FROM "Messages" WHERE "ChannelId" = old."Id";
    DELETE FROM "MessageChanges" WHERE "ChannelId" = old."Id";
END;
CREATE TRIGGER "ServerEmoji_Objects_AfterDelete"
AFTER DELETE ON "ServerEmoji"
BEGIN
    INSERT OR IGNORE INTO "ObjectDeletions" ("ObjectKey", "QueuedAt")
    VALUES (old."ObjectKey", strftime('%Y-%m-%d %H:%M:%f', 'now'));
END;
CREATE TABLE IF NOT EXISTS "OneTimeSteps" (
    "Name" TEXT NOT NULL CONSTRAINT "PK_OneTimeSteps" PRIMARY KEY,
    "AppliedAt" TEXT NOT NULL
);
