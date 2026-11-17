using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Data.Configurations;

internal sealed class NotificationDeliveryConfiguration : IEntityTypeConfiguration<NotificationDelivery>
{
    public void Configure(EntityTypeBuilder<NotificationDelivery> entity)
    {
        entity.HasKey(x => x.Id);

        // One row per message, recipient, event and destination. A second
        // enqueue for the same send is therefore impossible to store twice,
        // even if a caller is unsure whether its transaction committed.
        entity.HasIndex(x => new
            {
                x.SourceMessageId,
                x.RecipientId,
                x.EventKind,
                x.Channel,
                x.DestinationKey,
            })
            .IsUnique()
            .HasDatabaseName("IX_NotificationDeliveries_Unique");

        // The dispatcher's claim query: due rows first, then by age.
        entity.HasIndex(x => new { x.Status, x.NextAttemptAt, x.LeaseUntil });
        // Expiry and pruning.
        entity.HasIndex(x => x.ExpiresAt);
        // Finding the destinations of one recipient after an opt-out.
        entity.HasIndex(x => new { x.RecipientId, x.Channel });
        entity.HasIndex(x => x.PushSubscriptionId);

        entity.Property(x => x.EventKind).HasMaxLength(32);
        entity.Property(x => x.Channel).HasMaxLength(16);
        entity.Property(x => x.DestinationKey).HasMaxLength(320);
        entity.Property(x => x.Status).HasMaxLength(16);
        entity.Property(x => x.LastError).HasMaxLength(500).HasDefaultValue("");

        entity.HasOne(x => x.Recipient).WithMany().HasForeignKey(x => x.RecipientId)
            .OnDelete(DeleteBehavior.Cascade);
    }
}
