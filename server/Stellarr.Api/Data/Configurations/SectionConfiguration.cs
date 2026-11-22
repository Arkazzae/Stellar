using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Metadata.Builders;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Data.Configurations;

internal sealed class SectionConfiguration : IEntityTypeConfiguration<Section>
{
    public void Configure(EntityTypeBuilder<Section> entity)
    {
        entity.HasKey(x => x.Id);
        entity.HasIndex(x => new { x.ServerId, x.Position });
        entity.Property(x => x.Name).HasMaxLength(80);
        entity.HasOne<StellarrServer>().WithMany().HasForeignKey(x => x.ServerId)
            .OnDelete(DeleteBehavior.Cascade);
    }
}

internal sealed class PermissionOverrideConfiguration : IEntityTypeConfiguration<PermissionOverride>
{
    public void Configure(EntityTypeBuilder<PermissionOverride> entity)
    {
        entity.HasKey(x => x.Id);
        entity.HasIndex(x => new { x.TargetType, x.TargetId, x.RoleId }).IsUnique();
        entity.Property(x => x.AllowMask).HasConversion<long>();
        entity.Property(x => x.DenyMask).HasConversion<long>();
        entity.HasOne<StellarrServer>().WithMany().HasForeignKey(x => x.ServerId)
            .OnDelete(DeleteBehavior.Cascade);
        entity.HasOne<ServerRole>().WithMany().HasForeignKey(x => x.RoleId)
            .OnDelete(DeleteBehavior.Cascade);
    }
}

/// <remarks>
/// <c>MemberPermissionsSchema</c> creates the same table in a database that
/// predates it; a change here belongs there too.
/// </remarks>
internal sealed class MemberPermissionOverrideConfiguration : IEntityTypeConfiguration<MemberPermissionOverride>
{
    public void Configure(EntityTypeBuilder<MemberPermissionOverride> entity)
    {
        entity.HasKey(x => x.Id);
        entity.HasIndex(x => new { x.TargetType, x.TargetId, x.UserId }).IsUnique();
        entity.Property(x => x.AllowMask).HasConversion<long>();
        entity.Property(x => x.DenyMask).HasConversion<long>();
        entity.HasOne<ServerMember>().WithMany().HasForeignKey(x => new { x.ServerId, x.UserId })
            .OnDelete(DeleteBehavior.Cascade);
    }
}
