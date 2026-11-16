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
    /// Nothing is set for a busy database. Microsoft.Data.Sqlite retries a busy
    /// or locked statement itself, every 150 ms, until the command timeout:
    /// <c>Default Timeout</c>, 30 seconds when the string does not set it.
    /// SQLite's own <c>busy_timeout</c> is left at zero; it would only wait
    /// inside that loop and could neither lengthen nor shorten it.
    /// </para>
    /// </remarks>
    public static string From(string connectionString) =>
        new SqliteConnectionStringBuilder(connectionString) { ForeignKeys = true }.ToString();
}
