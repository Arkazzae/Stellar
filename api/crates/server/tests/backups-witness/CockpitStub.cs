using Microsoft.Data.Sqlite;

namespace Stellarr.Api.Endpoints;

// The two helpers of the first server's backup code that live in its endpoints
// class, copied as they are from Endpoints/CockpitEndpoints.Backups.cs
// (CopyDatabaseAsync, CheckIntegrityAsync, PlainFile). The rest of that class
// is the panel's and has nothing to do with archives.
public static class CockpitEndpoints
{
    internal static async Task CopyDatabaseAsync(
        string sourceConnectionString,
        string destinationPath,
        CancellationToken cancellationToken)
    {
        try
        {
            await using var source = new SqliteConnection(sourceConnectionString);
            await using var destination = new SqliteConnection(PlainFile(destinationPath, SqliteOpenMode.ReadWriteCreate));
            await source.OpenAsync(cancellationToken);
            await destination.OpenAsync(cancellationToken);
            source.BackupDatabase(destination);
        }
        catch
        {
            File.Delete(destinationPath);
            throw;
        }
    }

    internal static async Task<string?> CheckIntegrityAsync(string path, CancellationToken cancellationToken)
    {
        try
        {
            await using var connection = new SqliteConnection(PlainFile(path, SqliteOpenMode.ReadOnly));
            await connection.OpenAsync(cancellationToken);
            await using var command = connection.CreateCommand();
            command.CommandText = "PRAGMA integrity_check;";
            return await command.ExecuteScalarAsync(cancellationToken) as string;
        }
        catch (SqliteException exception)
        {
            return exception.Message;
        }
    }

    private static string PlainFile(string path, SqliteOpenMode mode) =>
        new SqliteConnectionStringBuilder { DataSource = path, Mode = mode, Pooling = false }.ToString();
}
