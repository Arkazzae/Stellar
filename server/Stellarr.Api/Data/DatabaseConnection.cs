using Microsoft.Data.Sqlite;

namespace Stellarr.Api.Data;

/// <summary>
/// The connection string every connection to the instance database opens
/// from: the API's pool, the backups it copies, and the CLI.
/// </summary>
public static class DatabaseConnection
{
    /// <summary>The configured connection string with foreign keys enforced.</summary>
    /// <remarks>
    /// <para>
    /// SQLite keeps <c>foreign_keys</c> per connection, and the pool opens as
    /// many as there are concurrent requests, so a pragma run once at start
    /// reaches one of them. With the keyword in the connection string,
    /// Microsoft.Data.Sqlite turns it on every time a connection is opened,
    /// pooled or new. The schema depends on it: deletes cascade in SQLite.
    /// </para>
    /// <para>
    /// A shared cache is refused. The database is in write-ahead logging, where
    /// a reader goes on from the last commit while another connection writes.
    /// A shared cache puts the connections of this process on one cache with
    /// table locks instead: a reader waits for a writer of the same table, and
    /// an online backup fails at once with SQLITE_BUSY while any of them has a
    /// write open.
    /// </para>
    /// <para>
    /// Nothing is set for a busy database. Microsoft.Data.Sqlite retries a busy
    /// or locked statement itself, every 150 ms, until the command timeout:
    /// <c>Default Timeout</c>, 30 seconds when the string does not set it.
    /// SQLite's own <c>busy_timeout</c> is left at zero; it would only wait
    /// inside that loop and could neither lengthen nor shorten it.
    /// </para>
    /// </remarks>
    /// <exception cref="InvalidOperationException">The connection string sets <c>Cache=Shared</c>.</exception>
    public static string From(string connectionString)
    {
        var settings = new SqliteConnectionStringBuilder(connectionString);
        if (settings.Cache == SqliteCacheMode.Shared)
        {
            throw new InvalidOperationException(
                "The database connection string sets Cache=Shared. The database runs in write-ahead logging, " +
                "which a shared cache defeats within this process; remove Cache=Shared.");
        }

        settings.ForeignKeys = true;
        return settings.ToString();
    }
}
