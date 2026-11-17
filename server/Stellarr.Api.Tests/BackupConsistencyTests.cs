using System.IO.Compression;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using Microsoft.Data.Sqlite;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Stellarr.Api.Backups;
using Stellarr.Api.Data;
using Stellarr.Api.Storage;

namespace Stellarr.Api.Tests;

public sealed class BackupConsistencyTests
{
    [Fact]
    public async Task AnObjectDeletedBetweenSnapshotAndListingIsReportedMissing()
    {
        var storage = new ListingGate(new FakeObjectStorage(), waitBeforeListing: true);
        using var factory = new StellarrWebApplicationFactory(true, configureServices: services =>
        {
            services.RemoveAll<IObjectStorage>();
            services.AddSingleton<IObjectStorage>(storage);
        });
        using var owner = factory.CreateClient();
        (await owner.PostAsJsonAsync("/api/auth/login", new { username = "maja", password = "Aurora!2026" })).EnsureSuccessStatusCode();
        var keys = new List<string>();
        for (var index = 0; index < 2; index++)
        {
            using var content = new ByteArrayContent("%PDF-1.7\n%%EOF\n"u8.ToArray());
            content.Headers.ContentType = new MediaTypeHeaderValue("application/pdf");
            using var form = new MultipartFormDataContent { { content, "file", "before-backup.pdf" } };
            var uploaded = await owner.PostAsync("/api/uploads", form);
            uploaded.EnsureSuccessStatusCode();
            var uploadId = (await uploaded.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetGuid();
            await using var scope = factory.Services.CreateAsyncScope();
            keys.Add((await scope.ServiceProvider.GetRequiredService<AppDbContext>().ImageAttachments.SingleAsync(x => x.Id == uploadId)).ObjectKey);
        }
        var start = await owner.PostAsJsonAsync("/api/cockpit/backups", new { database = true, objects = true, keys = true, instance = true });
        start.EnsureSuccessStatusCode();
        var id = (await start.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString()!;
        await storage.Listed.Task.WaitAsync(TimeSpan.FromSeconds(15));
        await storage.DeleteAsync(keys[0], CancellationToken.None);
        storage.Resume.SetResult();
        var deadline = DateTime.UtcNow.AddSeconds(30);
        while (factory.Services.GetRequiredService<BackupJobs>().Running is not null)
        {
            Assert.True(DateTime.UtcNow < deadline);
            await Task.Delay(25);
        }
        Assert.Null(factory.Services.GetRequiredService<BackupJobs>().LastFailure);
        var manifest = await BackupArchive.ReadManifestAsync(Path.Combine(factory.DataRoot, "backups", id), CancellationToken.None);
        Assert.Equal(keys[0], Assert.Single(manifest!.MissingObjects));
        Assert.Equal(1, manifest.ObjectCount);
    }

    [Fact]
    public async Task AnUploadCompletedAfterTheListingIsNotInTheEarlierDatabaseSnapshot()
    {
        var storage = new ListingGate(new FakeObjectStorage());
        using var factory = new StellarrWebApplicationFactory(true, configureServices: services =>
        {
            services.RemoveAll<IObjectStorage>();
            services.AddSingleton<IObjectStorage>(storage);
        });
        using var owner = factory.CreateClient();
        (await owner.PostAsJsonAsync("/api/auth/login", new { username = "maja", password = "Aurora!2026" })).EnsureSuccessStatusCode();
        var start = await owner.PostAsJsonAsync("/api/cockpit/backups", new { database = true, objects = true, keys = true, instance = true });
        start.EnsureSuccessStatusCode();
        var id = (await start.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetString()!;
        await storage.Listed.Task.WaitAsync(TimeSpan.FromSeconds(15));

        using var content = new ByteArrayContent("%PDF-1.7\n%%EOF\n"u8.ToArray());
        content.Headers.ContentType = new MediaTypeHeaderValue("application/pdf");
        using var form = new MultipartFormDataContent { { content, "file", "during-backup.pdf" } };
        var uploaded = await owner.PostAsync("/api/uploads", form);
        uploaded.EnsureSuccessStatusCode();
        var uploadId = (await uploaded.Content.ReadFromJsonAsync<JsonElement>()).GetProperty("id").GetGuid();
        string key;
        await using (var scope = factory.Services.CreateAsyncScope())
        {
            key = (await scope.ServiceProvider.GetRequiredService<AppDbContext>().ImageAttachments.SingleAsync(x => x.Id == uploadId)).ObjectKey;
        }
        storage.Resume.SetResult();
        var deadline = DateTime.UtcNow.AddSeconds(30);
        while (factory.Services.GetRequiredService<BackupJobs>().Running is not null)
        {
            Assert.True(DateTime.UtcNow < deadline);
            await Task.Delay(25);
        }
        Assert.Null(factory.Services.GetRequiredService<BackupJobs>().LastFailure);
        var archive = Path.Combine(factory.DataRoot, "backups", id);
        var manifest = await BackupArchive.ReadManifestAsync(archive, CancellationToken.None);
        Assert.Empty(manifest!.MissingObjects);
        Assert.Equal("ok", await BackupArchive.VerifyAsync(archive, factory.DataRoot, CancellationToken.None));
        using var zip = ZipFile.OpenRead(archive);
        Assert.Null(zip.GetEntry("objects/" + key));
        var snapshot = Path.Combine(factory.DataRoot, "proof.db");
        zip.GetEntry(BackupArchive.DatabaseEntry)!.ExtractToFile(snapshot);
        await using var connection = new SqliteConnection($"Data Source={snapshot};Pooling=False");
        await connection.OpenAsync();
        await using var query = connection.CreateCommand();
        query.CommandText = "SELECT count(*) FROM ImageAttachments WHERE ObjectKey=$key";
        query.Parameters.AddWithValue("$key", key);
        Assert.Equal(0L, await query.ExecuteScalarAsync());
    }

    [Fact]
    public async Task AFlippedObjectByteFailsVerificationAndCannotReplaceLiveData()
    {
        var root = Path.Combine(Path.GetTempPath(), $"stellarr-crc-proof-{Guid.NewGuid():N}");
        Directory.CreateDirectory(root);
        try
        {
            var database = Path.Combine(root, "source.db");
            await using (var connection = new SqliteConnection($"Data Source={database};Pooling=False"))
            {
                await connection.OpenAsync();
                await using var query = connection.CreateCommand();
                query.CommandText = "CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES ('source');";
                await query.ExecuteNonQueryAsync();
            }
            const string original = "unique-uncorrupted-object-payload-1234567890";
            var payload = Encoding.ASCII.GetBytes(original);
            var storage = new FakeObjectStorage();
            await storage.PutAsync("images/proof.png", new MemoryStream(payload), CancellationToken.None);
            var archive = Path.Combine(root, "archive.zip");
            await BackupArchive.WriteAsync(archive, BackupParts.All,
                new BackupSources($"Data Source={database};Pooling=False", storage,
                    [new StoredObjectInfo("images/proof.png", payload.Length)], Path.Combine(root, "keys"), "proof", [], root, root),
                new BackupProgress(), CancellationToken.None);
            var bytes = await File.ReadAllBytesAsync(archive);
            var offset = bytes.AsSpan().IndexOf(payload);
            Assert.True(offset >= 0);
            bytes[offset] ^= 1;
            await File.WriteAllBytesAsync(archive, bytes);
            using (var zip = ZipFile.OpenRead(archive))
            using (var reader = new StreamReader(zip.GetEntry("objects/images/proof.png")!.Open()))
                Assert.NotEqual(original, await reader.ReadToEndAsync());
            Assert.Contains("checksum", await BackupArchive.VerifyAsync(archive, root, CancellationToken.None), StringComparison.Ordinal);
            await Assert.ThrowsAsync<RestoreRefusedException>(() => BackupRestore.CheckAsync(archive, root, CancellationToken.None));
            var target = Path.Combine(root, "restored");
            Directory.CreateDirectory(target);
            var live = Path.Combine(target, "files", "images", "proof.png");
            Directory.CreateDirectory(Path.GetDirectoryName(live)!);
            await File.WriteAllTextAsync(live, original);
            await Assert.ThrowsAsync<RestoreRefusedException>(() => BackupRestore.ApplyAsync(archive, target, DateTime.UtcNow, CancellationToken.None));
            Assert.Equal(original, await File.ReadAllTextAsync(live));
        }
        finally { Directory.Delete(root, recursive: true); }
    }

    private sealed class ListingGate(FakeObjectStorage inner, bool waitBeforeListing = false) : IObjectStorage
    {
        public TaskCompletionSource Listed { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public TaskCompletionSource Resume { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public Task PrepareAsync(CancellationToken token) => inner.PrepareAsync(token);
        public Task PutAsync(string key, Stream data, CancellationToken token) => inner.PutAsync(key, data, token);
        public Task<StoredObject> GetAsync(string key, ByteRange? range, CancellationToken token) => inner.GetAsync(key, range, token);
        public Task DeleteAsync(string key, CancellationToken token) => inner.DeleteAsync(key, token);
        public Task ProbeAsync(CancellationToken token) => inner.ProbeAsync(token);
        public async IAsyncEnumerable<StoredObjectInfo> ListAsync([EnumeratorCancellation] CancellationToken token)
        {
            if (waitBeforeListing)
            {
                Listed.SetResult();
                await Resume.Task.WaitAsync(token);
            }
            await foreach (var item in inner.ListAsync(token)) yield return item;
            if (!waitBeforeListing)
            {
                Listed.SetResult();
                await Resume.Task.WaitAsync(token);
            }
        }
    }
}
