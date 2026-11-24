using Microsoft.Data.Sqlite;
using Stellarr.Api.Backups;
using Stellarr.Api.Storage;

// The first server's own backup code, run from a command line:
//   witness verify <archive> <work-folder>
//   witness check  <archive> <work-folder>
//   witness apply  <archive> <data-root>
//   witness make   <data-root> <parts d|o|k|i|all> <out.zip>
var cancel = CancellationToken.None;
try
{
    switch (args[0])
    {
        case "verify":
            Directory.CreateDirectory(args[2]);
            var verdict = await BackupArchive.VerifyAsync(args[1], args[2], cancel);
            Console.WriteLine(verdict);
            return verdict == "ok" ? 0 : 1;
        case "check":
            var manifest = await BackupRestore.CheckAsync(args[1], args[2], cancel);
            Console.WriteLine("ok " + manifest.InstanceName + " objects=" + manifest.ObjectCount);
            return 0;
        case "apply":
            var result = await BackupRestore.ApplyAsync(args[1], args[2], DateTime.UtcNow, cancel);
            Console.WriteLine("restored; previous data in " + result.PreviousDataDirectory);
            return 0;
        case "make":
            var root = Path.GetFullPath(args[1]);
            var parts = args[2] == "all" ? BackupParts.All : args[2].Aggregate(BackupParts.None, (all, letter) => all | letter switch
            {
                'd' => BackupParts.Database, 'o' => BackupParts.Objects, 'k' => BackupParts.Keys, 'i' => BackupParts.Instance,
                _ => throw new ArgumentException("part " + letter),
            });
            var storage = new FileSystemObjectStorage(Path.Combine(root, FileSystemObjectStorage.FolderName));
            var objects = new List<StoredObjectInfo>();
            await foreach (var item in storage.ListAsync(cancel)) objects.Add(item);
            objects = objects.OrderBy(x => x.Key, StringComparer.Ordinal).ToList();
            var database = Path.Combine(root, "db", "stellarr.db");
            string name;
            await using (var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = database, Mode = SqliteOpenMode.ReadOnly, Pooling = false }.ToString()))
            {
                await connection.OpenAsync(cancel);
                await using var command = connection.CreateCommand();
                command.CommandText = "SELECT \"Name\" FROM \"Servers\" ORDER BY \"CreatedAt\", \"Id\" LIMIT 1";
                name = (string)(await command.ExecuteScalarAsync(cancel))!;
            }
            var work = Path.GetDirectoryName(Path.GetFullPath(args[3]))!;
            await BackupArchive.WriteAsync(args[3], parts,
                new BackupSources("Data Source=" + database, storage, objects, Path.Combine(root, "keys"), name, [], work, root),
                new BackupProgress { ObjectsTotal = objects.Count }, cancel);
            return 0;
    }
    Console.Error.WriteLine("verify | check | apply | make");
    return 2;
}
catch (Exception exception)
{
    Console.Error.WriteLine(exception is RestoreRefusedException or RestoreBrokenException
        ? "REFUSED " + exception.Message
        : "ERROR " + exception.GetType().Name + " " + exception.Message);
    return 3;
}
