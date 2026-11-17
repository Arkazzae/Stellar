using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Data.Configurations;

/// <summary>
/// One row per browser subscription. The endpoint is unique across the whole
/// instance: a subscription is never silently handed from one account or
/// sign-in to another.
/// </summary>
internal sealed class PushSubscriptionConfiguration : IEntityTypeConfiguration<PushSubscriptionRecord>
{
    internal const int EndpointMaxLength = 2_048;
    internal const int P256dhMaxLength = 256;
    internal const int AuthMaxLength = 64;

    public void Configure(EntityTypeBuilder<PushSubscriptionRecord> entity)
    {
        entity.ToTable("PushSubscriptions");
        entity.HasKey(x => x.Id);
        entity.HasIndex(x => x.Endpoint).IsUnique();
        entity.HasIndex(x => new { x.UserId, x.SessionId });
        entity.Property(x => x.Endpoint).HasMaxLength(EndpointMaxLength);
        entity.Property(x => x.P256dh).HasMaxLength(P256dhMaxLength);
        entity.Property(x => x.Auth).HasMaxLength(AuthMaxLength);
        entity.HasOne<User>().WithMany().HasForeignKey(x => x.UserId)
            .OnDelete(DeleteBehavior.Cascade);
    }
}
