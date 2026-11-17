using System.Buffers;
using System.IO.Compression;

namespace Stellarr.Api.Backups;

/// <summary>Checks the ZIP's CRC-32 and uncompressed length while copying an entry.</summary>
internal static class BackupEntryIntegrity
{
    private static readonly uint[] Table = CreateTable();

    internal static async Task CopyAsync(ZipArchiveEntry entry, Stream target, CancellationToken cancellationToken)
    {
        var buffer = ArrayPool<byte>.Shared.Rent(81920);
        try
        {
            await using var source = await entry.OpenAsync(cancellationToken);
            var crc = uint.MaxValue;
            long length = 0;
            int read;
            while ((read = await source.ReadAsync(buffer.AsMemory(), cancellationToken)) > 0)
            {
                for (var index = 0; index < read; index++)
                {
                    crc = Table[(crc ^ buffer[index]) & 0xff] ^ (crc >> 8);
                }
                length += read;
                await target.WriteAsync(buffer.AsMemory(0, read), cancellationToken);
            }

            if (length != entry.Length || (crc ^ uint.MaxValue) != entry.Crc32)
            {
                throw new InvalidDataException($"The archive entry '{entry.FullName}' failed its checksum or length check.");
            }
        }
        finally
        {
            ArrayPool<byte>.Shared.Return(buffer);
        }
    }

    private static uint[] CreateTable()
    {
        var table = new uint[256];
        for (uint index = 0; index < table.Length; index++)
        {
            var crc = index;
            for (var bit = 0; bit < 8; bit++)
            {
                crc = (crc >> 1) ^ ((crc & 1) == 0 ? 0 : 0xedb88320u);
            }
            table[index] = crc;
        }
        return table;
    }
}
