using System.Buffers.Binary;
using System.Buffers.Text;

namespace Stellarr.Api.Messaging;

/// <summary>
/// A snapshot ceiling and the last delivered key. Deleting any delivered row,
/// including the anchor, cannot move the next page past an unread message.
/// </summary>
internal readonly record struct HistoryCursor(
    Guid ScopeId, DateTime Snapshot, DateTime? BeforeCreatedAt = null, Guid? BeforeId = null)
{
    private const byte Version = 1;
    private const int LegacySize = 28;
    private const int Size = 49;

    public string Encode()
    {
        if (BeforeCreatedAt is not { } before || BeforeId is not { } id)
        {
            throw new InvalidOperationException("A continuation cursor requires a message key.");
        }

        Span<byte> bytes = stackalloc byte[Size];
        bytes[0] = Version;
        ScopeId.TryWriteBytes(bytes[1..17]);
        BinaryPrimitives.WriteInt64BigEndian(bytes[17..25], Snapshot.Ticks);
        BinaryPrimitives.WriteInt64BigEndian(bytes[25..33], before.Ticks);
        id.TryWriteBytes(bytes[33..]);
        return Base64Url.EncodeToString(bytes);
    }

    public static bool TryDecode(string encoded, out HistoryCursor cursor)
    {
        cursor = default;
        if (!Base64Url.IsValid(encoded, out var length) || (length != Size && length != LegacySize))
        {
            return false;
        }

        Span<byte> bytes = stackalloc byte[length];
        Base64Url.DecodeFromChars(encoded, bytes);
        if (length == LegacySize)
        {
            var snapshot = BinaryPrimitives.ReadInt64BigEndian(bytes[16..24]);
            var offset = BinaryPrimitives.ReadInt32BigEndian(bytes[24..]);
            if (!ValidTicks(snapshot) || offset < 0)
            {
                return false;
            }

            // Old cursors cannot identify their last row after a deletion.
            // Restart their original snapshot once, allowing clients to merge
            // already-seen IDs, instead of silently skipping unread rows.
            cursor = new HistoryCursor(new Guid(bytes[..16]), new DateTime(snapshot, DateTimeKind.Utc));
            return true;
        }

        var ticks = BinaryPrimitives.ReadInt64BigEndian(bytes[17..25]);
        var beforeTicks = BinaryPrimitives.ReadInt64BigEndian(bytes[25..33]);
        if (bytes[0] != Version || !ValidTicks(ticks) || !ValidTicks(beforeTicks) || beforeTicks > ticks)
        {
            return false;
        }

        cursor = new HistoryCursor(
            new Guid(bytes[1..17]), new DateTime(ticks, DateTimeKind.Utc),
            new DateTime(beforeTicks, DateTimeKind.Utc), new Guid(bytes[33..]));
        return true;
    }

    private static bool ValidTicks(long ticks) => ticks >= 0 && ticks <= DateTime.MaxValue.Ticks;
}
