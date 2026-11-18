using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Data.Configurations;

internal sealed class AccountKeyConfiguration : IEntityTypeConfiguration<AccountKey>
{
    public void Configure(EntityTypeBuilder<AccountKey> entity)
    {
        entity.HasKey(x => x.UserId);
        entity.HasIndex(x => x.KeyId).IsUnique();
        entity.Property(x => x.Algorithm).HasMaxLength(32);
        entity.HasOne(x => x.User).WithMany().HasForeignKey(x => x.UserId)
            .OnDelete(DeleteBehavior.Cascade);
    }
}

internal sealed class AccountKeyBackupConfiguration : IEntityTypeConfiguration<AccountKeyBackup>
{
    public void Configure(EntityTypeBuilder<AccountKeyBackup> entity)
    {
        entity.HasKey(x => x.UserId);
        entity.HasOne(x => x.User).WithMany().HasForeignKey(x => x.UserId)
            .OnDelete(DeleteBehavior.Cascade);
    }
}

internal sealed class EncryptionDeviceConfiguration : IEntityTypeConfiguration<EncryptionDevice>
{
    public void Configure(EntityTypeBuilder<EncryptionDevice> entity)
    {
        entity.HasKey(x => x.Id);
        entity.HasIndex(x => x.UserId);
        // One device with keys per login session; a replaced one is revoked first.
        entity.HasIndex(x => x.SessionId).IsUnique().HasFilter("\"RevokedAt\" IS NULL");
        entity.Property(x => x.Algorithm).HasMaxLength(32);
        entity.Property(x => x.Label).HasMaxLength(EncryptionLimits.MaxDeviceLabelLength);
        entity.HasOne<User>().WithMany().HasForeignKey(x => x.UserId)
            .OnDelete(DeleteBehavior.Cascade);
        entity.HasOne(x => x.Session).WithMany().HasForeignKey(x => x.SessionId)
            .OnDelete(DeleteBehavior.Cascade);
    }
}

internal sealed class EncryptionEpochConfiguration : IEntityTypeConfiguration<EncryptionEpoch>
{
    public void Configure(EntityTypeBuilder<EncryptionEpoch> entity)
    {
        entity.HasKey(x => x.Id);
        entity.HasIndex(x => new { x.ConversationId, x.Sequence }).IsUnique();
        entity.HasOne<DirectConversation>().WithMany().HasForeignKey(x => x.ConversationId)
            .OnDelete(DeleteBehavior.Cascade);
        entity.HasOne<User>().WithMany().HasForeignKey(x => x.CreatedById)
            .OnDelete(DeleteBehavior.Cascade);
    }
}

internal sealed class EncryptionEnvelopeConfiguration : IEntityTypeConfiguration<EncryptionEnvelope>
{
    public void Configure(EntityTypeBuilder<EncryptionEnvelope> entity)
    {
        entity.HasKey(x => new { x.EpochId, x.RecipientKeyId });
        entity.Property(x => x.RecipientKind).HasMaxLength(16);
        entity.HasOne<EncryptionEpoch>().WithMany(x => x.Envelopes).HasForeignKey(x => x.EpochId)
            .OnDelete(DeleteBehavior.Cascade);
    }
}
