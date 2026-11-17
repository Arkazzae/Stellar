namespace Stellarr.Api.Domain;

/// <summary>The last successful send, independent of whether its message still exists.</summary>
public sealed class ChannelPostTime
{
    public Guid ChannelId { get; set; }
    public Guid UserId { get; set; }
    public DateTime LastPostedAt { get; set; }
}
