using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Data.Configurations;

// DiscordImportSchema creates the same tables in a database that predates
// them; a change here belongs there too. No key here may make an ordinary
// delete fail: a deleted server, channel, section or message clears the
// bookkeeping's pointer to it, or takes the rows that only make sense with it.

/// <summary>A Discord id as the reader gives it: a snowflake of up to 20 digits.</summary>
internal static class DiscordIdColumn
{
    internal const int MaxLength = 20;
}

internal sealed class DiscordImportConfiguration : IEntityTypeConfiguration<DiscordImport>
{
    public void Configure(EntityTypeBuilder<DiscordImport> entity)
    {
        entity.HasKey(x => x.Id);
        entity.HasIndex(x => x.ServerId);
        entity.Property(x => x.DiscordServerId).HasMaxLength(DiscordIdColumn.MaxLength);
        entity.Property(x => x.DiscordServerName).HasMaxLength(100);
        // The record stays when the server goes: its placeholders do not.
        entity.HasOne<StellarrServer>().WithMany().HasForeignKey(x => x.ServerId)
            .OnDelete(DeleteBehavior.SetNull);
        entity.HasOne<User>().WithMany().HasForeignKey(x => x.StartedById)
            .OnDelete(DeleteBehavior.SetNull);
    }
}

internal sealed class DiscordImportAccountConfiguration : IEntityTypeConfiguration<DiscordImportAccount>
{
    public void Configure(EntityTypeBuilder<DiscordImportAccount> entity)
    {
        entity.HasKey(x => x.DiscordId);
        entity.HasIndex(x => x.UserId).IsUnique();
        entity.HasIndex(x => x.ImportId);
        entity.Property(x => x.DiscordId).HasMaxLength(DiscordIdColumn.MaxLength);
        entity.Property(x => x.DiscordUsername).HasMaxLength(100);
        entity.Property(x => x.DiscordAvatarUrl).HasMaxLength(256);
        entity.HasOne<User>().WithMany().HasForeignKey(x => x.UserId)
            .OnDelete(DeleteBehavior.Cascade);
        entity.HasOne<DiscordImport>().WithMany().HasForeignKey(x => x.ImportId)
            .OnDelete(DeleteBehavior.SetNull);
    }
}

internal sealed class DiscordImportSectionConfiguration : IEntityTypeConfiguration<DiscordImportSection>
{
    public void Configure(EntityTypeBuilder<DiscordImportSection> entity)
    {
        entity.HasKey(x => new { x.ServerId, x.DiscordId });
        entity.HasIndex(x => x.ImportId);
        entity.HasIndex(x => x.SectionId);
        entity.Property(x => x.DiscordId).HasMaxLength(DiscordIdColumn.MaxLength);
        entity.HasOne<StellarrServer>().WithMany().HasForeignKey(x => x.ServerId)
            .OnDelete(DeleteBehavior.Cascade);
        entity.HasOne<DiscordImport>().WithMany().HasForeignKey(x => x.ImportId)
            .OnDelete(DeleteBehavior.Cascade);
        entity.HasOne<Section>().WithMany().HasForeignKey(x => x.SectionId)
            .OnDelete(DeleteBehavior.SetNull);
    }
}

internal sealed class DiscordImportChannelConfiguration : IEntityTypeConfiguration<DiscordImportChannel>
{
    public void Configure(EntityTypeBuilder<DiscordImportChannel> entity)
    {
        entity.HasKey(x => new { x.ServerId, x.DiscordId });
        entity.HasIndex(x => x.ImportId);
        entity.HasIndex(x => x.ChannelId);
        entity.HasIndex(x => new { x.ServerId, x.ParentDiscordId });
        entity.Property(x => x.DiscordId).HasMaxLength(DiscordIdColumn.MaxLength);
        entity.Property(x => x.ParentDiscordId).HasMaxLength(DiscordIdColumn.MaxLength);
        entity.Property(x => x.StarterMessageId).HasMaxLength(DiscordIdColumn.MaxLength);
        entity.Property(x => x.OldestMessageId).HasMaxLength(DiscordIdColumn.MaxLength);
        entity.Property(x => x.Name).HasMaxLength(100);
        entity.HasOne<StellarrServer>().WithMany().HasForeignKey(x => x.ServerId)
            .OnDelete(DeleteBehavior.Cascade);
        entity.HasOne<DiscordImport>().WithMany().HasForeignKey(x => x.ImportId)
            .OnDelete(DeleteBehavior.Cascade);
        entity.HasOne<Channel>().WithMany().HasForeignKey(x => x.ChannelId)
            .OnDelete(DeleteBehavior.SetNull);
        // Not a key: the root is one of the thread's messages, and a deleted
        // root is chosen again when the thread is linked again.
    }
}

internal sealed class DiscordImportMessageConfiguration : IEntityTypeConfiguration<DiscordImportMessage>
{
    public void Configure(EntityTypeBuilder<DiscordImportMessage> entity)
    {
        entity.HasKey(x => new { x.ServerId, x.DiscordId });
        entity.HasIndex(x => x.ImportId);
        entity.HasIndex(x => x.MessageId);
        // Linking reads one channel's messages, and the counts group them by outcome.
        entity.HasIndex(x => new { x.ServerId, x.ChannelDiscordId, x.Outcome });
        entity.Property(x => x.DiscordId).HasMaxLength(DiscordIdColumn.MaxLength);
        entity.Property(x => x.ChannelDiscordId).HasMaxLength(DiscordIdColumn.MaxLength);
        entity.Property(x => x.ReplyToDiscordId).HasMaxLength(DiscordIdColumn.MaxLength);
        entity.HasOne<StellarrServer>().WithMany().HasForeignKey(x => x.ServerId)
            .OnDelete(DeleteBehavior.Cascade);
        entity.HasOne<DiscordImport>().WithMany().HasForeignKey(x => x.ImportId)
            .OnDelete(DeleteBehavior.Cascade);
        entity.HasOne<Message>().WithMany().HasForeignKey(x => x.MessageId)
            .OnDelete(DeleteBehavior.SetNull);
    }
}
