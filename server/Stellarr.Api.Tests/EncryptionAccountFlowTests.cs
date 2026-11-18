using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Stellarr.Api.Domain;
using static Stellarr.Api.Tests.EncryptionTestKit;

namespace Stellarr.Api.Tests;

/// <summary>
/// The account's side of encryption: its key, the PIN backup and the
/// attempts on it, and the browsers that hold keys.
/// </summary>
public sealed class EncryptionAccountFlowTests
{
    private const string BackupUrl = "/api/me/encryption/backup";
    private const string UnlockUrl = "/api/me/encryption/backup/unlock";

    [Fact]
    public async Task AnAccountSetsUpOneKeyWithAValidBackup()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await SignInAsync(factory, "maja");

        var shortKey = await maja.PostAsJsonAsync("/api/me/encryption/account-key", new
        {
            algorithm = "x25519",
            publicKey = Bytes(31),
            backup = Backup(Bytes(32)),
        });
        var unknownAlgorithm = await maja.PostAsJsonAsync("/api/me/encryption/account-key", new
        {
            algorithm = "rsa",
            publicKey = Bytes(32),
            backup = Backup(Bytes(32)),
        });
        var weakBackup = await maja.PostAsJsonAsync("/api/me/encryption/account-key", new
        {
            algorithm = "x25519",
            publicKey = Bytes(32),
            backup = new { salt = Bytes(16), iterations = 1000, verifier = Bytes(32), iv = Bytes(12), ciphertext = Bytes(64) },
        });
        Assert.All([shortKey, unknownAlgorithm, weakBackup], x => Assert.Equal(HttpStatusCode.BadRequest, x.StatusCode));
        Assert.Equal(
            JsonValueKind.Null,
            (await maja.GetFromJsonAsync<JsonElement>("/api/me/encryption")).GetProperty("accountKey").ValueKind);

        var account = await SetUpAsync(maja);
        var again = await maja.PostAsJsonAsync("/api/me/encryption/account-key", new
        {
            algorithm = "x25519",
            publicKey = Bytes(32),
            backup = Backup(Bytes(32)),
        });
        Assert.Equal((HttpStatusCode.Conflict, "account-key-exists"), await RefusalAsync(again));

        var state = await maja.GetFromJsonAsync<JsonElement>("/api/me/encryption");
        Assert.Equal(account.KeyId, state.GetProperty("accountKey").GetProperty("keyId").GetString());
        Assert.Equal("x25519", state.GetProperty("accountKey").GetProperty("algorithm").GetString());
        Assert.Equal(10, state.GetProperty("backup").GetProperty("remainingAttempts").GetInt32());
        Assert.Equal(JsonValueKind.Null, state.GetProperty("device").ValueKind);
    }

    [Fact]
    public async Task WrongPinsAreCountedAndTheTenthInARowDeletesTheBackupButNotTheKey()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await SignInAsync(factory, "maja");
        var account = await SetUpAsync(maja);

        var parameters = await maja.GetFromJsonAsync<JsonElement>(BackupUrl);
        Assert.Equal(16, parameters.GetProperty("salt").GetBytesFromBase64().Length);
        Assert.Equal(600_000, parameters.GetProperty("iterations").GetInt32());
        Assert.Equal(10, parameters.GetProperty("remainingAttempts").GetInt32());
        Assert.Equal(HttpStatusCode.BadRequest, (await maja.PostAsJsonAsync(UnlockUrl, new { authKey = Bytes(16) })).StatusCode);

        var wrong = await maja.PostAsJsonAsync(UnlockUrl, new { authKey = Bytes(32) });
        Assert.Equal((HttpStatusCode.Forbidden, "pin-incorrect"), await RefusalAsync(wrong));
        Assert.Equal(9, (await JsonAsync(wrong)).GetProperty("remainingAttempts").GetInt32());

        // A right PIN hands out the backup and forgets the wrong ones.
        var right = await maja.PostAsJsonAsync(UnlockUrl, new { authKey = account.AuthKey });
        Assert.Equal(HttpStatusCode.OK, right.StatusCode);
        var unlocked = await JsonAsync(right);
        Assert.Equal(account.KeyId, unlocked.GetProperty("accountKeyId").GetString());
        Assert.Equal(64, unlocked.GetProperty("ciphertext").GetBytesFromBase64().Length);
        Assert.Equal(12, unlocked.GetProperty("iv").GetBytesFromBase64().Length);
        Assert.Equal(10, (await maja.GetFromJsonAsync<JsonElement>(BackupUrl)).GetProperty("remainingAttempts").GetInt32());

        var remaining = new List<int>();
        for (var attempt = 0; attempt < 9; attempt++)
        {
            var response = await maja.PostAsJsonAsync(UnlockUrl, new { authKey = Bytes(32) });
            Assert.Equal(HttpStatusCode.Forbidden, response.StatusCode);
            remaining.Add((await JsonAsync(response)).GetProperty("remainingAttempts").GetInt32());
        }

        Assert.Equal([9, 8, 7, 6, 5, 4, 3, 2, 1], remaining);
        var tenth = await maja.PostAsJsonAsync(UnlockUrl, new { authKey = Bytes(32) });
        Assert.Equal((HttpStatusCode.Gone, "backup-deleted"), await RefusalAsync(tenth));
        Assert.Equal(0, (await JsonAsync(tenth)).GetProperty("remainingAttempts").GetInt32());

        Assert.Equal(
            (HttpStatusCode.NotFound, "backup-missing"),
            await RefusalAsync(await maja.PostAsJsonAsync(UnlockUrl, new { authKey = account.AuthKey })));
        Assert.Equal((HttpStatusCode.NotFound, "backup-missing"), await RefusalAsync(await maja.GetAsync(BackupUrl)));
        var state = await maja.GetFromJsonAsync<JsonElement>("/api/me/encryption");
        Assert.Equal(account.KeyId, state.GetProperty("accountKey").GetProperty("keyId").GetString());
        Assert.Equal(JsonValueKind.Null, state.GetProperty("backup").ValueKind);
    }

    [Fact]
    public async Task AttemptsSentAtOnceAreCountedOneAfterAnother()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await SignInAsync(factory, "maja");
        await SetUpAsync(maja);

        var responses = await Task.WhenAll(Enumerable.Range(0, 12)
            .Select(_ => maja.PostAsJsonAsync(UnlockUrl, new { authKey = Bytes(32) })));

        var statuses = responses.Select(x => x.StatusCode).ToArray();
        Assert.Equal(9, statuses.Count(x => x == HttpStatusCode.Forbidden));
        Assert.Equal(1, statuses.Count(x => x == HttpStatusCode.Gone));
        Assert.Equal(2, statuses.Count(x => x == HttpStatusCode.NotFound));
        var remaining = new List<int>();
        foreach (var response in responses.Where(x => x.StatusCode == HttpStatusCode.Forbidden))
        {
            remaining.Add((await JsonAsync(response)).GetProperty("remainingAttempts").GetInt32());
        }

        Assert.Equal([1, 2, 3, 4, 5, 6, 7, 8, 9], remaining.Order());
    }

    [Fact]
    public async Task PinAttemptsAreLimitedPerAccountAndPerAddress()
    {
        using var factory = new StellarrWebApplicationFactory(
            loadTestData: true,
            configureServices: services => services.AddSingleton<IStartupFilter>(new PeerFromHeader()));
        // The limits are lifted in the Testing environment, so this host runs
        // as development, where they come from configuration.
        using var limited = factory.WithWebHostBuilder(builder =>
        {
            builder.UseEnvironment("Development");
            builder.UseSetting("RateLimiting:PinUnlock:PermitLimit", "2");
        });
        using var maja = await SignInAsync(limited, "maja");
        using var ada = await SignInAsync(limited, "ada");
        await SetUpAsync(maja);
        await SetUpAsync(ada);

        Task<HttpResponseMessage> Attempt(HttpClient client, string address)
        {
            var request = new HttpRequestMessage(HttpMethod.Post, UnlockUrl)
            {
                Content = JsonContent.Create(new { authKey = Bytes(32) }),
            };
            request.Headers.Add(PeerFromHeader.Header, address);
            return client.SendAsync(request);
        }

        Assert.Equal(HttpStatusCode.Forbidden, (await Attempt(maja, "10.0.0.1")).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await Attempt(maja, "10.0.0.1")).StatusCode);
        // The account has used its tries, from wherever the next one comes.
        Assert.Equal(HttpStatusCode.TooManyRequests, (await Attempt(maja, "10.0.0.1")).StatusCode);
        Assert.Equal(HttpStatusCode.TooManyRequests, (await Attempt(maja, "10.0.0.2")).StatusCode);
        // Another account from that second address still gets one, and then
        // the address has used its tries too.
        Assert.Equal(HttpStatusCode.Forbidden, (await Attempt(ada, "10.0.0.2")).StatusCode);
        Assert.Equal(HttpStatusCode.TooManyRequests, (await Attempt(ada, "10.0.0.2")).StatusCode);
        Assert.Equal(HttpStatusCode.Forbidden, (await Attempt(ada, "10.0.0.3")).StatusCode);

        // Refused attempts are not counted against the backup.
        Assert.Equal(8, (await maja.GetFromJsonAsync<JsonElement>(BackupUrl)).GetProperty("remainingAttempts").GetInt32());
    }

    [Fact]
    public async Task ANewPinNeedsThePasswordAFreshSecondStepAndABrowserThatHoldsTheKey()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await SignInAsync(factory, "maja");
        var account = await SetUpAsync(maja);
        var newAuthKey = Bytes(32);
        object NewPin(string password, string keyId) => new
        {
            currentPassword = password,
            accountKeyId = keyId,
            backup = Backup(newAuthKey),
        };

        Assert.Equal(
            (HttpStatusCode.Forbidden, "password-incorrect"),
            await RefusalAsync(await maja.PutAsJsonAsync(BackupUrl, NewPin("not-the-password", account.KeyId))));
        Assert.Equal(
            (HttpStatusCode.Conflict, "device-missing"),
            await RefusalAsync(await maja.PutAsJsonAsync(BackupUrl, NewPin(People["maja"].Password, account.KeyId))));
        await RegisterDeviceAsync(maja);
        Assert.Equal(
            (HttpStatusCode.Conflict, "account-key-changed"),
            await RefusalAsync(await maja.PutAsJsonAsync(BackupUrl, NewPin(People["maja"].Password, Guid.NewGuid().ToString()))));

        // A new PIN also brings back a backup that wrong PINs deleted.
        for (var attempt = 0; attempt < 10; attempt++)
        {
            await maja.PostAsJsonAsync(UnlockUrl, new { authKey = Bytes(32) });
        }

        Assert.Equal(HttpStatusCode.NotFound, (await maja.GetAsync(BackupUrl)).StatusCode);
        var replaced = await maja.PutAsJsonAsync(BackupUrl, NewPin(People["maja"].Password, account.KeyId));
        Assert.Equal(HttpStatusCode.OK, replaced.StatusCode);
        Assert.Equal(10, (await JsonAsync(replaced)).GetProperty("remainingAttempts").GetInt32());
        Assert.Equal(
            (HttpStatusCode.Forbidden, "pin-incorrect"),
            await RefusalAsync(await maja.PostAsJsonAsync(UnlockUrl, new { authKey = account.AuthKey })));
        Assert.Equal(HttpStatusCode.OK, (await maja.PostAsJsonAsync(UnlockUrl, new { authKey = newAuthKey })).StatusCode);

        // Once the account has a second step, it has to be fresh.
        await AddTotpAsync(factory, People["maja"].Id);
        await SetSecondStepAsync(factory, People["maja"].Id, DateTime.UtcNow.AddMinutes(-15));
        Assert.Equal(
            (HttpStatusCode.Forbidden, "fresh-mfa-required"),
            await RefusalAsync(await maja.PutAsJsonAsync(BackupUrl, NewPin(People["maja"].Password, account.KeyId))));
        await SetSecondStepAsync(factory, People["maja"].Id, DateTime.UtcNow);
        Assert.Equal(HttpStatusCode.OK, (await maja.PutAsJsonAsync(BackupUrl, NewPin(People["maja"].Password, account.KeyId))).StatusCode);
    }

    [Fact]
    public async Task AResetReplacesTheKeyRevokesTheOtherDevicesAndTellsTheEncryptedConversations()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var majaLaptop = await SignInAsync(factory, "maja");
        await RegisterDeviceAsync(majaLaptop);
        using var ada = await ReadyAsync(factory, "ada");
        using var ilya = await ReadyAsync(factory, "ilya");
        var withAda = await OpenAsync(maja, "ada");
        var withIlya = await OpenAsync(maja, "ilya");
        await EnableAsync(maja, ada, withAda);
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(maja, withIlya, "request")).StatusCode);
        var oldKeyId = (await maja.GetFromJsonAsync<JsonElement>("/api/me/encryption"))
            .GetProperty("accountKey").GetProperty("keyId").GetString();
        var ownDevice = (await maja.GetFromJsonAsync<JsonElement>("/api/me/encryption"))
            .GetProperty("device").GetProperty("id").GetString();
        object Reset(string password) => new
        {
            currentPassword = password,
            algorithm = "x25519",
            publicKey = Bytes(32),
            backup = Backup(Bytes(32)),
        };

        Assert.Equal(
            (HttpStatusCode.Forbidden, "password-incorrect"),
            await RefusalAsync(await maja.PostAsJsonAsync("/api/me/encryption/account-key/reset", Reset("not-the-password"))));
        await AddTotpAsync(factory, People["maja"].Id);
        await SetSecondStepAsync(factory, People["maja"].Id, DateTime.UtcNow.AddMinutes(-15));
        Assert.Equal(
            (HttpStatusCode.Forbidden, "fresh-mfa-required"),
            await RefusalAsync(await maja.PostAsJsonAsync("/api/me/encryption/account-key/reset", Reset(People["maja"].Password))));
        await SetSecondStepAsync(factory, People["maja"].Id, DateTime.UtcNow);

        var reset = await maja.PostAsJsonAsync("/api/me/encryption/account-key/reset", Reset(People["maja"].Password));
        Assert.Equal(HttpStatusCode.OK, reset.StatusCode);
        var result = await JsonAsync(reset);
        var newKeyId = result.GetProperty("accountKey").GetProperty("keyId").GetString();
        Assert.NotEqual(oldKeyId, newKeyId);
        Assert.Equal(1, result.GetProperty("revokedDevices").GetInt32());

        var seenByAda = await KeysAsync(ada, withAda);
        Assert.Equal(newKeyId, seenByAda.GetProperty("peer").GetProperty("accountKey").GetProperty("keyId").GetString());
        Assert.Equal([ownDevice], seenByAda.GetProperty("peer").GetProperty("devices").EnumerateArray().Select(x => x.GetProperty("keyId").GetString()));
        Assert.Contains("keys_reset:maja", await TimelineAsync(ada, withAda));
        Assert.DoesNotContain("keys_reset:maja", await TimelineAsync(ilya, withIlya));

        // The laptop stays signed in, without keys until it registers again.
        var laptop = await majaLaptop.GetAsync("/api/me/encryption");
        Assert.Equal(HttpStatusCode.OK, laptop.StatusCode);
        Assert.Equal(JsonValueKind.Null, (await JsonAsync(laptop)).GetProperty("device").ValueKind);
    }

    [Fact]
    public async Task ADeviceLivesAsLongAsItsSessionAndRegisteringItAgainChangesNothing()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await SignInAsync(factory, "maja");
        Assert.Equal((HttpStatusCode.Conflict, "account-key-missing"), await RefusalAsync(await RegisterDeviceResponseAsync(maja)));
        await SetUpAsync(maja);
        using var ada = await ReadyAsync(factory, "ada");
        var conversationId = await OpenAsync(maja, "ada");
        await EnableAsync(maja, ada, conversationId);

        var publicKey = Bytes(32);
        var first = await RegisterDeviceResponseAsync(maja, publicKey);
        Assert.Equal(HttpStatusCode.Created, first.StatusCode);
        var deviceId = (await JsonAsync(first)).GetProperty("id").GetString()!;
        var again = await RegisterDeviceResponseAsync(maja, publicKey, "Firefox, Linux (renamed)");
        Assert.Equal(HttpStatusCode.OK, again.StatusCode);
        Assert.Equal(deviceId, (await JsonAsync(again)).GetProperty("id").GetString());
        Assert.Equal(HttpStatusCode.BadRequest, (await RegisterDeviceResponseAsync(maja, label: " ")).StatusCode);

        // A new key for the same browser replaces its device.
        var replaced = await RegisterDeviceResponseAsync(maja);
        Assert.Equal(HttpStatusCode.Created, replaced.StatusCode);
        deviceId = (await JsonAsync(replaced)).GetProperty("id").GetString()!;
        Assert.Equal(["device_added:maja", "device_added:maja"], (await TimelineAsync(ada, conversationId)).Where(x => x.StartsWith("device_added", StringComparison.Ordinal)));

        using var laptop = await SignInAsync(factory, "maja");
        var laptopDevice = await RegisterDeviceAsync(laptop);
        using var phone = await SignInAsync(factory, "maja");
        var phoneDevice = await RegisterDeviceAsync(phone);
        using var tablet = await SignInAsync(factory, "maja");
        var tabletDevice = await RegisterDeviceAsync(tablet);
        Assert.Equal([deviceId, laptopDevice, phoneDevice, tabletDevice], await PeerDevicesAsync(ada, conversationId));
        var own = await maja.GetFromJsonAsync<JsonElement>("/api/me/encryption/devices");
        Assert.Equal(
            [true, false, false, false],
            own.GetProperty("devices").EnumerateArray().Select(x => x.GetProperty("current").GetBoolean()));

        // Signing out, a session revoked from the list, and removing the keys all end a device.
        Assert.Equal(HttpStatusCode.NoContent, (await laptop.PostAsync("/api/auth/logout", null)).StatusCode);
        Guid phoneSession;
        await using (var scope = factory.OpenScope(out var db))
        {
            phoneSession = await db.EncryptionDevices.Where(x => x.Id == Guid.Parse(phoneDevice)).Select(x => x.SessionId).SingleAsync();
        }

        Assert.Equal(HttpStatusCode.NoContent, (await maja.DeleteAsync($"/api/me/security/sessions/{phoneSession}")).StatusCode);
        Assert.Equal(HttpStatusCode.NoContent, (await maja.DeleteAsync($"/api/me/encryption/devices/{tabletDevice}")).StatusCode);
        Assert.Equal(HttpStatusCode.Unauthorized, (await tablet.GetAsync("/api/me")).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await maja.DeleteAsync($"/api/me/encryption/devices/{tabletDevice}")).StatusCode);
        Assert.Equal(
            (HttpStatusCode.Conflict, "current-device"),
            await RefusalAsync(await maja.DeleteAsync($"/api/me/encryption/devices/{deviceId}")));
        Assert.Equal(HttpStatusCode.NotFound, (await ada.DeleteAsync($"/api/me/encryption/devices/{deviceId}")).StatusCode);

        Assert.Equal([deviceId], await PeerDevicesAsync(ada, conversationId));
        Assert.Single((await maja.GetFromJsonAsync<JsonElement>("/api/me/encryption/devices")).GetProperty("devices").EnumerateArray());
    }

    [Fact]
    public async Task AnAccountHoldsKeysInABoundedNumberOfBrowsers()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await SignInAsync(factory, "maja");
        await SetUpAsync(maja);
        await using (var scope = factory.OpenScope(out var db))
        {
            var now = DateTime.UtcNow;
            for (var index = 0; index < EncryptionLimits.MaxActiveDevices; index++)
            {
                var session = new UserSession
                {
                    Id = Guid.NewGuid(),
                    UserId = People["maja"].Id,
                    Device = "Seeded",
                    IpAddress = "203.0.113.7",
                    CreatedAt = now,
                    LastSeenAt = now,
                    ExpiresAt = now.AddDays(1),
                    AbsoluteExpiresAt = now.AddDays(1),
                };
                db.UserSessions.Add(session);
                db.EncryptionDevices.Add(new EncryptionDevice
                {
                    Id = Guid.NewGuid(),
                    UserId = People["maja"].Id,
                    SessionId = session.Id,
                    Algorithm = EncryptionAlgorithms.X25519,
                    PublicKey = Bytes(32),
                    Label = "Seeded",
                    CreatedAt = now.AddSeconds(-index),
                    LastSeenAt = now,
                });
            }

            await db.SaveChangesAsync();
        }

        var refused = await RegisterDeviceResponseAsync(maja);
        Assert.Equal((HttpStatusCode.Conflict, "device-limit"), await RefusalAsync(refused));
        Assert.Equal(EncryptionLimits.MaxActiveDevices, (await JsonAsync(refused)).GetProperty("max").GetInt32());

        var oldest = (await maja.GetFromJsonAsync<JsonElement>("/api/me/encryption/devices"))
            .GetProperty("devices")[0].GetProperty("id").GetString();
        Assert.Equal(HttpStatusCode.NoContent, (await maja.DeleteAsync($"/api/me/encryption/devices/{oldest}")).StatusCode);
        Assert.Equal(HttpStatusCode.Created, (await RegisterDeviceResponseAsync(maja)).StatusCode);
    }

    [Fact]
    public async Task ANewDeviceIsAnnouncedOnlyInTheAccountsEncryptedConversations()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        using var ilya = await ReadyAsync(factory, "ilya");
        var majaAda = await OpenAsync(maja, "ada");
        var majaIlya = await OpenAsync(maja, "ilya");
        var adaIlya = await OpenAsync(ada, "ilya");
        await EnableAsync(maja, ada, majaAda);
        await EnableAsync(ada, ilya, adaIlya);
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(maja, majaIlya, "request")).StatusCode);

        using var laptop = await SignInAsync(factory, "maja");
        var publicKey = Bytes(32);
        Assert.Equal(HttpStatusCode.Created, (await RegisterDeviceResponseAsync(laptop, publicKey)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await RegisterDeviceResponseAsync(laptop, publicKey)).StatusCode);

        Assert.Equal(["encryption_requested:maja", "encryption_enabled:ada", "device_added:maja"], await TimelineAsync(ada, majaAda));
        Assert.Equal(["encryption_requested:maja"], await TimelineAsync(ilya, majaIlya));
        Assert.Equal(["encryption_requested:ada", "encryption_enabled:ilya"], await TimelineAsync(ilya, adaIlya));
    }

    private static async Task<string[]> PeerDevicesAsync(HttpClient client, string conversationId) =>
        [.. (await KeysAsync(client, conversationId)).GetProperty("peer").GetProperty("devices").EnumerateArray()
            .Select(x => x.GetProperty("keyId").GetString()!)];

    private static async Task AddTotpAsync(StellarrWebApplicationFactory factory, Guid userId)
    {
        await using var scope = factory.OpenScope(out var db);
        db.MfaMethods.Add(new MfaMethod
        {
            Id = Guid.NewGuid(),
            UserId = userId,
            Type = MfaMethodTypes.Totp,
            Name = "Seeded",
            CreatedAt = DateTime.UtcNow,
            ConfirmedAt = DateTime.UtcNow,
            LastTotpStep = 1,
        });
        await db.SaveChangesAsync();
    }

    private static async Task SetSecondStepAsync(StellarrWebApplicationFactory factory, Guid userId, DateTime completedAt)
    {
        await using var scope = factory.OpenScope(out var db);
        await db.UserSessions.Where(x => x.UserId == userId && x.RevokedAt == null)
            .ExecuteUpdateAsync(setters => setters.SetProperty(x => x.MfaCompletedAt, completedAt));
    }

    /// <summary>Lets a test say which address a request comes from.</summary>
    private sealed class PeerFromHeader : IStartupFilter
    {
        internal const string Header = "X-Test-Peer";

        public Action<IApplicationBuilder> Configure(Action<IApplicationBuilder> next) => app =>
        {
            app.Use((context, following) =>
            {
                if (IPAddress.TryParse(context.Request.Headers[Header].ToString(), out var peer))
                {
                    context.Connection.RemoteIpAddress = peer;
                }

                return following(context);
            });
            next(app);
        };
    }
}
