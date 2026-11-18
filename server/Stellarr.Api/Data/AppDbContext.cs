using Microsoft.EntityFrameworkCore;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Data;

/// <remarks>
/// The DbSet property names are the table names, which the SQL in
/// <c>Data/Schema</c> and the CLI's schema check name too. Entity mappings live
/// in <c>Data/Configurations</c>.
/// </remarks>
public sealed class AppDbContext(DbContextOptions<AppDbContext> options) : DbContext(options)
{
    public DbSet<User> Users => Set<User>();
    public DbSet<PasswordResetToken> PasswordResetTokens => Set<PasswordResetToken>();
    public DbSet<EmailChangeRequest> EmailChangeRequests => Set<EmailChangeRequest>();
    public DbSet<ExternalIdentity> ExternalIdentities => Set<ExternalIdentity>();
    public DbSet<PendingExternalIdentity> PendingExternalIdentities => Set<PendingExternalIdentity>();
    public DbSet<UserPreferenceSettings> UserPreferences => Set<UserPreferenceSettings>();
    public DbSet<UserSession> UserSessions => Set<UserSession>();
    public DbSet<DesktopBrowserLogin> DesktopBrowserLogins => Set<DesktopBrowserLogin>();
    public DbSet<MfaMethod> MfaMethods => Set<MfaMethod>();
    public DbSet<RecoveryCode> RecoveryCodes => Set<RecoveryCode>();
    public DbSet<TrustedDevice> TrustedDevices => Set<TrustedDevice>();
    public DbSet<AuthChallenge> AuthChallenges => Set<AuthChallenge>();
    public DbSet<WebAuthnCredential> WebAuthnCredentials => Set<WebAuthnCredential>();
    public DbSet<PasskeySignIn> PasskeySignIns => Set<PasskeySignIn>();
    public DbSet<EmailOtpCode> EmailOtpCodes => Set<EmailOtpCode>();
    public DbSet<CliRecoveryCredential> CliRecoveryCredentials => Set<CliRecoveryCredential>();
    public DbSet<SecurityEvent> SecurityEvents => Set<SecurityEvent>();
    public DbSet<SecurityNotification> SecurityNotifications => Set<SecurityNotification>();
    public DbSet<StellarrServer> Servers => Set<StellarrServer>();
    public DbSet<OperationsSettings> OperationsSettings => Set<OperationsSettings>();
    public DbSet<InstanceSecret> InstanceSecrets => Set<InstanceSecret>();
    public DbSet<IdentityProviderSetting> IdentityProviderSettings => Set<IdentityProviderSetting>();
    public DbSet<SoundboardSound> SoundboardSounds => Set<SoundboardSound>();
    public DbSet<ServerRole> ServerRoles => Set<ServerRole>();
    public DbSet<ServerMember> ServerMembers => Set<ServerMember>();
    public DbSet<ServerBan> ServerBans => Set<ServerBan>();
    public DbSet<ServerInvite> ServerInvites => Set<ServerInvite>();
    public DbSet<ServerAuditEntry> ServerAuditEntries => Set<ServerAuditEntry>();
    public DbSet<OperatorAuditEntry> OperatorAuditEntries => Set<OperatorAuditEntry>();
    public DbSet<ServerAdminSettings> ServerAdminSettings => Set<ServerAdminSettings>();
    public DbSet<Channel> Channels => Set<Channel>();
    public DbSet<ChannelRead> ChannelReads => Set<ChannelRead>();
    public DbSet<ChannelPostTime> ChannelPostTimes => Set<ChannelPostTime>();
    public DbSet<Section> Sections => Set<Section>();
    public DbSet<PermissionOverride> PermissionOverrides => Set<PermissionOverride>();
    public DbSet<ServerEmoji> ServerEmoji => Set<ServerEmoji>();
    public DbSet<Message> Messages => Set<Message>();
    public DbSet<MessageChange> MessageChanges => Set<MessageChange>();
    public DbSet<MessageIndexEntry> MessageIndexEntries => Set<MessageIndexEntry>();
    public DbSet<MessageReaction> MessageReactions => Set<MessageReaction>();
    public DbSet<ImageAttachment> ImageAttachments => Set<ImageAttachment>();
    public DbSet<ObjectDeletion> ObjectDeletions => Set<ObjectDeletion>();
    public DbSet<MessageAttachment> MessageAttachments => Set<MessageAttachment>();
    public DbSet<MessageEmbed> MessageEmbeds => Set<MessageEmbed>();
    public DbSet<UserNotification> UserNotifications => Set<UserNotification>();
    public DbSet<NotificationDelivery> NotificationDeliveries => Set<NotificationDelivery>();
    public DbSet<PushSubscriptionRecord> PushSubscriptions => Set<PushSubscriptionRecord>();
    public DbSet<Friendship> Friendships => Set<Friendship>();
    public DbSet<FriendRequest> FriendRequests => Set<FriendRequest>();
    public DbSet<UserBlock> UserBlocks => Set<UserBlock>();
    public DbSet<DirectConversation> DirectConversations => Set<DirectConversation>();
    public DbSet<DirectMessage> DirectMessages => Set<DirectMessage>();
    public DbSet<DirectMessageAttachment> DirectMessageAttachments => Set<DirectMessageAttachment>();
    public DbSet<DirectConversationRead> DirectConversationReads => Set<DirectConversationRead>();
    public DbSet<AccountKey> AccountKeys => Set<AccountKey>();
    public DbSet<AccountKeyBackup> AccountKeyBackups => Set<AccountKeyBackup>();
    public DbSet<EncryptionDevice> EncryptionDevices => Set<EncryptionDevice>();
    public DbSet<EncryptionEpoch> EncryptionEpochs => Set<EncryptionEpoch>();
    public DbSet<EncryptionEnvelope> EncryptionEnvelopes => Set<EncryptionEnvelope>();

    protected override void OnModelCreating(ModelBuilder modelBuilder) =>
        modelBuilder.ApplyConfigurationsFromAssembly(typeof(AppDbContext).Assembly);
}
