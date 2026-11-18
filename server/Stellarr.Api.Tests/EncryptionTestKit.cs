using System.Net;
using System.Net.Http.Json;
using System.Security.Cryptography;
using System.Text.Json;
using Microsoft.AspNetCore.Http.Connections;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.AspNetCore.SignalR.Client;

namespace Stellarr.Api.Tests;

/// <summary>
/// What the encryption tests do the way a browser would, with random bytes in
/// place of real keys and ciphertext: the server never opens either.
/// </summary>
internal static class EncryptionTestKit
{
    internal static readonly Dictionary<string, (Guid Id, string Password)> People = new()
    {
        ["maja"] = (Guid.Parse("30000000-0000-0000-0000-000000000001"), "Aurora!2026"),
        ["ada"] = (Guid.Parse("30000000-0000-0000-0000-000000000002"), "Quasar!2026"),
        ["ilya"] = (Guid.Parse("30000000-0000-0000-0000-000000000003"), "Meteor!2026"),
    };

    /// <summary>The account key's id and the PIN proof its backup was made with.</summary>
    internal sealed record Account(string KeyId, byte[] AuthKey);

    internal static byte[] Bytes(int length) => RandomNumberGenerator.GetBytes(length);

    internal static async Task<HttpClient> SignInAsync(WebApplicationFactory<Program> factory, string username)
    {
        var client = factory.CreateClient(new WebApplicationFactoryClientOptions
        {
            AllowAutoRedirect = false,
            HandleCookies = true,
        });
        (await client.PostAsJsonAsync("/api/auth/login", new { username, password = People[username].Password }))
            .EnsureSuccessStatusCode();
        return client;
    }

    /// <summary>Signed in, with an account key and this browser's device.</summary>
    internal static async Task<HttpClient> ReadyAsync(WebApplicationFactory<Program> factory, string username)
    {
        var client = await SignInAsync(factory, username);
        await SetUpAsync(client);
        await RegisterDeviceAsync(client);
        return client;
    }

    internal static async Task<string> OpenAsync(HttpClient client, string username)
    {
        var response = await client.PostAsJsonAsync("/api/social/conversations", new { userId = People[username].Id });
        response.EnsureSuccessStatusCode();
        return (await JsonAsync(response)).GetProperty("id").GetString()!;
    }

    internal static object Backup(byte[] authKey) => new
    {
        salt = Bytes(16),
        iterations = 600_000,
        verifier = SHA256.HashData(authKey),
        iv = Bytes(12),
        ciphertext = Bytes(64),
    };

    internal static async Task<Account> SetUpAsync(HttpClient client)
    {
        var authKey = Bytes(32);
        var response = await client.PostAsJsonAsync("/api/me/encryption/account-key", new
        {
            algorithm = "x25519",
            publicKey = Bytes(32),
            backup = Backup(authKey),
        });
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        var keyId = (await JsonAsync(response)).GetProperty("accountKey").GetProperty("keyId").GetString()!;
        return new Account(keyId, authKey);
    }

    internal static Task<HttpResponseMessage> RegisterDeviceResponseAsync(
        HttpClient client,
        byte[]? publicKey = null,
        string label = "Firefox, Linux") =>
        client.PostAsJsonAsync("/api/me/encryption/devices", new
        {
            algorithm = "x25519",
            publicKey = publicKey ?? Bytes(32),
            label,
        });

    internal static async Task<string> RegisterDeviceAsync(HttpClient client)
    {
        var response = await RegisterDeviceResponseAsync(client);
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        return (await JsonAsync(response)).GetProperty("id").GetString()!;
    }

    internal static Task<HttpResponseMessage> SwitchAsync(HttpClient client, string conversationId, string action) =>
        client.PostAsync($"/api/social/conversations/{conversationId}/encryption/{action}", null);

    /// <summary>One person asks, the other confirms.</summary>
    internal static async Task EnableAsync(HttpClient requester, HttpClient confirmer, string conversationId)
    {
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(requester, conversationId, "request")).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(confirmer, conversationId, "confirm")).StatusCode);
    }

    internal static async Task<JsonElement> KeysAsync(HttpClient client, string conversationId) =>
        await client.GetFromJsonAsync<JsonElement>($"/api/social/conversations/{conversationId}/encryption/keys");

    internal static string[] RecipientIds(JsonElement keys) =>
        [.. keys.GetProperty("recipients").EnumerateArray().Select(x => x.GetProperty("keyId").GetString()!)];

    internal static Task<HttpResponseMessage> PostEpochAsync(
        HttpClient client,
        string conversationId,
        string epochId,
        string? previousEpochId,
        IEnumerable<string> keyIds) =>
        client.PostAsJsonAsync($"/api/social/conversations/{conversationId}/encryption/epochs", new
        {
            epochId,
            previousEpochId,
            envelopes = keyIds.Select(keyId => new
            {
                recipientKeyId = keyId,
                ephemeralPublicKey = Bytes(32),
                iv = Bytes(12),
                ciphertext = Bytes(48),
            }),
        });

    /// <summary>Starts a new epoch for exactly the keys the conversation has now, as a browser would.</summary>
    internal static async Task<string> RotateAsync(HttpClient client, string conversationId)
    {
        var keys = await KeysAsync(client, conversationId);
        var epochId = Guid.NewGuid().ToString();
        var previous = keys.GetProperty("encryption").GetProperty("currentEpochId").GetString();
        var response = await PostEpochAsync(client, conversationId, epochId, previous, RecipientIds(keys));
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        return epochId;
    }

    internal static Task<HttpResponseMessage> SendEncryptedAsync(
        HttpClient client,
        string conversationId,
        string epochId,
        Guid? clientMessageId = null,
        IEnumerable<string>? attachmentIds = null,
        byte[]? ciphertext = null) =>
        client.PostAsJsonAsync($"/api/social/conversations/{conversationId}/messages", new
        {
            clientMessageId = clientMessageId ?? Guid.NewGuid(),
            attachmentIds = attachmentIds ?? [],
            encrypted = new { epochId, iv = Bytes(12), ciphertext = ciphertext ?? Bytes(80) },
        });

    internal static Task<HttpResponseMessage> SendPlainAsync(
        HttpClient client,
        string conversationId,
        string content,
        IEnumerable<string>? attachmentIds = null,
        Guid? clientMessageId = null) =>
        client.PostAsJsonAsync($"/api/social/conversations/{conversationId}/messages", new
        {
            content,
            attachmentIds = attachmentIds ?? [],
            clientMessageId,
        });

    internal static async Task<JsonElement[]> HistoryAsync(HttpClient client, string conversationId) =>
        [.. (await client.GetFromJsonAsync<JsonElement>($"/api/social/conversations/{conversationId}/messages"))
            .GetProperty("messages").EnumerateArray()];

    /// <summary>The timeline lines of a conversation, oldest first, as "kind:actor".</summary>
    internal static async Task<string[]> TimelineAsync(HttpClient client, string conversationId) =>
        [.. (await HistoryAsync(client, conversationId))
            .Where(x => x.GetProperty("system").ValueKind == JsonValueKind.Object)
            .Select(x => $"{x.GetProperty("system").GetProperty("kind").GetString()}:{Name(x.GetProperty("system").GetProperty("actorId").GetString()!)}")];

    internal static string Name(string userId) => People.Single(x => x.Value.Id.ToString() == userId).Key;

    internal static async Task<JsonElement> JsonAsync(HttpResponseMessage response) =>
        await response.Content.ReadFromJsonAsync<JsonElement>();

    /// <summary>The status and the problem code of a refusal.</summary>
    internal static async Task<(HttpStatusCode Status, string? Code)> RefusalAsync(HttpResponseMessage response)
    {
        var body = await response.Content.ReadAsStringAsync();
        string? code = null;
        if (body.Length > 0)
        {
            using var document = JsonDocument.Parse(body);
            if (document.RootElement.ValueKind == JsonValueKind.Object &&
                document.RootElement.TryGetProperty("code", out var value))
            {
                code = value.GetString();
            }
        }

        return (response.StatusCode, code);
    }

    internal static async Task<string> UploadEncryptedAsync(HttpClient client, byte[]? content = null)
    {
        using var form = new MultipartFormDataContent();
        var file = new ByteArrayContent(content ?? Bytes(300));
        file.Headers.ContentType = new System.Net.Http.Headers.MediaTypeHeaderValue("application/octet-stream");
        form.Add(file, "file", "blob");
        var response = await client.PostAsync("/api/uploads/encrypted", form);
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        return (await JsonAsync(response)).GetProperty("id").GetString()!;
    }

    internal static async Task<string> UploadPngAsync(HttpClient client)
    {
        using var form = new MultipartFormDataContent();
        var image = new ByteArrayContent([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0]);
        image.Headers.ContentType = new System.Net.Http.Headers.MediaTypeHeaderValue("image/png");
        form.Add(image, "file", "tiny.png");
        var response = await client.PostAsync("/api/uploads", form);
        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        return (await JsonAsync(response)).GetProperty("id").GetString()!;
    }

    /// <summary>A hub connection of a session of its own, for listening.</summary>
    internal static async Task<HubConnection> ListenAsync(WebApplicationFactory<Program> factory, string username)
    {
        using var client = factory.CreateClient(new WebApplicationFactoryClientOptions
        {
            AllowAutoRedirect = false,
            HandleCookies = false,
        });
        var login = await client.PostAsJsonAsync("/api/auth/login", new { username, password = People[username].Password });
        login.EnsureSuccessStatusCode();
        var cookie = string.Join("; ", login.Headers.GetValues("Set-Cookie").Select(x => x.Split(';', 2)[0]));
        var connection = new HubConnectionBuilder()
            .WithUrl("https://localhost/hubs/app", options =>
            {
                options.Headers["Cookie"] = cookie;
                options.Headers["X-Stellarr-Client"] = "web";
                options.Headers["X-Stellarr-Client-Version"] = "2.0.0";
                options.Transports = HttpTransportType.LongPolling;
                options.HttpMessageHandlerFactory = _ => factory.Server.CreateHandler();
            })
            .Build();
        return connection;
    }
}
