using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.SignalR.Client;
using static Stellarr.Api.Tests.EncryptionTestKit;

namespace Stellarr.Api.Tests;

/// <summary>
/// The encryption of one private conversation: the switch both people agree
/// on, the epochs, and which messages and uploads the conversation takes in
/// each state. Each test runs against an application of its own.
/// </summary>
public sealed class DirectEncryptionFlowTests
{
    [Fact]
    public async Task EncryptionIsOnOnlyOnceTheOtherPersonConfirmsWithAKeyOfTheirOwn()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await SignInAsync(factory, "maja");
        using var ada = await SignInAsync(factory, "ada");
        var conversationId = await OpenAsync(maja, "ada");

        Assert.Equal((HttpStatusCode.Conflict, "account-key-missing"), await RefusalAsync(await SwitchAsync(maja, conversationId, "request")));
        await SetUpAsync(maja);
        var requested = await SwitchAsync(maja, conversationId, "request");
        Assert.Equal(HttpStatusCode.OK, requested.StatusCode);
        var state = await JsonAsync(requested);
        Assert.Equal("requested", state.GetProperty("state").GetString());
        Assert.Equal(People["maja"].Id.ToString(), state.GetProperty("requestedById").GetString());

        var twice = await SwitchAsync(maja, conversationId, "request");
        Assert.Equal((HttpStatusCode.Conflict, "encryption-state"), await RefusalAsync(twice));
        Assert.Equal("requested", (await JsonAsync(twice)).GetProperty("encryption").GetProperty("state").GetString());
        Assert.Equal((HttpStatusCode.Forbidden, "encryption-not-allowed"), await RefusalAsync(await SwitchAsync(maja, conversationId, "confirm")));
        Assert.Equal((HttpStatusCode.Conflict, "account-key-missing"), await RefusalAsync(await SwitchAsync(ada, conversationId, "confirm")));

        await SetUpAsync(ada);
        var confirmed = await SwitchAsync(ada, conversationId, "confirm");
        Assert.Equal(HttpStatusCode.OK, confirmed.StatusCode);
        state = await JsonAsync(confirmed);
        Assert.Equal("on", state.GetProperty("state").GetString());
        Assert.Equal(JsonValueKind.String, state.GetProperty("enabledAt").ValueKind);
        Assert.Equal(JsonValueKind.Null, state.GetProperty("currentEpochId").ValueKind);
        Assert.Equal(JsonValueKind.Null, state.GetProperty("requestedById").ValueKind);

        var listed = Assert.Single((await maja.GetFromJsonAsync<JsonElement>("/api/social")).GetProperty("conversations").EnumerateArray());
        Assert.Equal("on", listed.GetProperty("encryption").GetProperty("state").GetString());
        Assert.Equal(["encryption_requested:maja", "encryption_enabled:ada"], await TimelineAsync(ada, conversationId));
    }

    [Fact]
    public async Task OnlyTheRequesterCancelsEitherPersonSwitchesOffAndOnAgainTakesBothAgain()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await SignInAsync(factory, "maja");
        using var ada = await SignInAsync(factory, "ada");
        await SetUpAsync(maja);
        await SetUpAsync(ada);
        var conversationId = await OpenAsync(maja, "ada");

        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(maja, conversationId, "request")).StatusCode);
        Assert.Equal((HttpStatusCode.Forbidden, "encryption-not-allowed"), await RefusalAsync(await SwitchAsync(ada, conversationId, "cancel")));
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(maja, conversationId, "cancel")).StatusCode);
        Assert.Equal((HttpStatusCode.Conflict, "encryption-state"), await RefusalAsync(await SwitchAsync(maja, conversationId, "cancel")));

        // Whoever confirmed may switch it off as well as whoever asked.
        await EnableAsync(ada, maja, conversationId);
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(maja, conversationId, "disable")).StatusCode);
        Assert.Equal((HttpStatusCode.Conflict, "encryption-state"), await RefusalAsync(await SwitchAsync(ada, conversationId, "confirm")));
        Assert.Equal((HttpStatusCode.Conflict, "encryption-state"), await RefusalAsync(await SwitchAsync(ada, conversationId, "disable")));

        await EnableAsync(maja, ada, conversationId);
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(ada, conversationId, "disable")).StatusCode);

        // Turning a request down is switching off before it was ever on.
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(maja, conversationId, "request")).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(ada, conversationId, "disable")).StatusCode);

        Assert.Equal(
            [
                "encryption_requested:maja", "encryption_request_cancelled:maja",
                "encryption_requested:ada", "encryption_enabled:maja", "encryption_disabled:maja",
                "encryption_requested:maja", "encryption_enabled:ada", "encryption_disabled:ada",
                "encryption_requested:maja", "encryption_disabled:ada",
            ],
            await TimelineAsync(maja, conversationId));
    }

    [Fact]
    public async Task SomebodyWhoTakesMessagesOnlyFromFriendsTakesRequestsOnlyFromThemToo()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        (await ada.PatchAsJsonAsync("/api/me/preferences", new { allowDirectMessages = false })).EnsureSuccessStatusCode();
        var conversationId = await OpenAsync(maja, "ada");

        Assert.Equal(HttpStatusCode.Forbidden, (await SwitchAsync(maja, conversationId, "request")).StatusCode);
        Assert.Empty(await TimelineAsync(ada, conversationId));

        (await maja.PostAsJsonAsync("/api/social/requests", new { username = "ada" })).EnsureSuccessStatusCode();
        var request = Assert.Single((await ada.GetFromJsonAsync<JsonElement>("/api/social")).GetProperty("requests").EnumerateArray());
        (await ada.PostAsync($"/api/social/requests/{request.GetProperty("id").GetString()}/accept", null)).EnsureSuccessStatusCode();
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(maja, conversationId, "request")).StatusCode);
    }

    [Fact]
    public async Task OutsidersFindNothingAndABlockClosesTheEncryptionToo()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        using var ilya = await ReadyAsync(factory, "ilya");
        var conversationId = await OpenAsync(maja, "ada");
        await EnableAsync(maja, ada, conversationId);
        var epochId = await RotateAsync(maja, conversationId);

        foreach (var action in new[] { "request", "cancel", "confirm", "disable" })
        {
            Assert.Equal(HttpStatusCode.NotFound, (await SwitchAsync(ilya, conversationId, action)).StatusCode);
        }

        Assert.Equal(HttpStatusCode.NotFound, (await ilya.GetAsync($"/api/social/conversations/{conversationId}/encryption/keys")).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await ilya.GetAsync($"/api/social/conversations/{conversationId}/encryption/epochs")).StatusCode);
        Assert.Equal(
            HttpStatusCode.NotFound,
            (await PostEpochAsync(ilya, conversationId, Guid.NewGuid().ToString(), epochId, [Guid.NewGuid().ToString()])).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await SendEncryptedAsync(ilya, conversationId, epochId)).StatusCode);

        (await maja.PostAsync($"/api/social/blocks/{People["ada"].Id}", null)).EnsureSuccessStatusCode();
        Assert.Equal(HttpStatusCode.Forbidden, (await SwitchAsync(ada, conversationId, "disable")).StatusCode);
        Assert.Equal(HttpStatusCode.NotFound, (await ada.GetAsync($"/api/social/conversations/{conversationId}/encryption/keys")).StatusCode);
        Assert.Equal(
            HttpStatusCode.Forbidden,
            (await PostEpochAsync(ada, conversationId, Guid.NewGuid().ToString(), epochId, [Guid.NewGuid().ToString()])).StatusCode);
    }

    [Fact]
    public async Task AnEpochFollowsTheCurrentOneAndReachesExactlyEveryKeyOfBothPeople()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        var conversationId = await OpenAsync(maja, "ada");

        Assert.Equal(
            (HttpStatusCode.Conflict, "encryption-off"),
            await RefusalAsync(await PostEpochAsync(maja, conversationId, Guid.NewGuid().ToString(), null, [Guid.NewGuid().ToString()])));
        await EnableAsync(maja, ada, conversationId);

        var keys = await KeysAsync(maja, conversationId);
        var everyKey = RecipientIds(keys);
        Assert.Equal(4, everyKey.Length);
        var adaDevice = keys.GetProperty("peer").GetProperty("devices")[0].GetProperty("keyId").GetString()!;
        var stranger = Guid.NewGuid().ToString();

        var stale = await PostEpochAsync(maja, conversationId, Guid.NewGuid().ToString(), Guid.NewGuid().ToString(), everyKey);
        Assert.Equal((HttpStatusCode.Conflict, "epoch-stale"), await RefusalAsync(stale));

        var uncovered = await PostEpochAsync(
            maja, conversationId, Guid.NewGuid().ToString(), null, [.. everyKey.Where(x => x != adaDevice), stranger]);
        Assert.Equal(HttpStatusCode.Conflict, uncovered.StatusCode);
        var coverage = await JsonAsync(uncovered);
        Assert.Equal("epoch-coverage", coverage.GetProperty("code").GetString());
        Assert.Equal([adaDevice], coverage.GetProperty("missing").EnumerateArray().Select(x => x.GetString()));
        Assert.Equal([stranger], coverage.GetProperty("extra").EnumerateArray().Select(x => x.GetString()));

        Assert.Equal(
            HttpStatusCode.BadRequest,
            (await PostEpochAsync(maja, conversationId, Guid.NewGuid().ToString().ToUpperInvariant(), null, everyKey)).StatusCode);

        var first = Guid.NewGuid().ToString();
        var created = await PostEpochAsync(maja, conversationId, first, null, everyKey);
        Assert.Equal(HttpStatusCode.Created, created.StatusCode);
        Assert.Equal(1, (await JsonAsync(created)).GetProperty("sequence").GetInt32());
        Assert.Equal(HttpStatusCode.OK, (await PostEpochAsync(maja, conversationId, first, null, everyKey)).StatusCode);
        Assert.Equal((HttpStatusCode.Conflict, "epoch-exists"), await RefusalAsync(await PostEpochAsync(ada, conversationId, first, first, everyKey)));

        var late = await PostEpochAsync(ada, conversationId, Guid.NewGuid().ToString(), null, everyKey);
        Assert.Equal((HttpStatusCode.Conflict, "epoch-stale"), await RefusalAsync(late));
        var second = Guid.NewGuid().ToString();
        Assert.Equal(HttpStatusCode.Created, (await PostEpochAsync(ada, conversationId, second, first, everyKey)).StatusCode);

        // Each reader gets the envelopes for their own account key and browser, and no others.
        var epochs = await maja.GetFromJsonAsync<JsonElement>($"/api/social/conversations/{conversationId}/encryption/epochs");
        Assert.Equal(second, epochs.GetProperty("currentEpochId").GetString());
        Assert.Equal([first, second], epochs.GetProperty("epochs").EnumerateArray().Select(x => x.GetProperty("id").GetString()));
        var majaKeys = new[]
        {
            keys.GetProperty("self").GetProperty("accountKey").GetProperty("keyId").GetString(),
            keys.GetProperty("currentDeviceKeyId").GetString(),
        }.Order().ToArray();
        foreach (var epoch in epochs.GetProperty("epochs").EnumerateArray())
        {
            Assert.Equal(4, epoch.GetProperty("recipients").GetArrayLength());
            Assert.Equal(majaKeys, epoch.GetProperty("envelopes").EnumerateArray().Select(x => x.GetProperty("recipientKeyId").GetString()).Order());
        }

        // Switching on again starts over from no epoch, and the old ones stay readable.
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(ada, conversationId, "disable")).StatusCode);
        Assert.Equal((HttpStatusCode.Conflict, "encryption-off"), await RefusalAsync(await PostEpochAsync(maja, conversationId, Guid.NewGuid().ToString(), second, everyKey)));
        await EnableAsync(ada, maja, conversationId);
        var third = Guid.NewGuid().ToString();
        var restarted = await PostEpochAsync(maja, conversationId, third, null, everyKey);
        Assert.Equal(HttpStatusCode.Created, restarted.StatusCode);
        Assert.Equal(3, (await JsonAsync(restarted)).GetProperty("sequence").GetInt32());
        var later = await ada.GetFromJsonAsync<JsonElement>($"/api/social/conversations/{conversationId}/encryption/epochs?after=1");
        Assert.Equal([second, third], later.GetProperty("epochs").EnumerateArray().Select(x => x.GetProperty("id").GetString()));
    }

    [Fact]
    public async Task WhileOnOnlyMessagesEncryptedWithTheCurrentEpochGetIn()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        var conversationId = await OpenAsync(maja, "ada");

        Assert.Equal(
            (HttpStatusCode.Conflict, "encryption-off"),
            await RefusalAsync(await SendEncryptedAsync(maja, conversationId, Guid.NewGuid().ToString())));
        await EnableAsync(maja, ada, conversationId);

        Assert.Equal((HttpStatusCode.Conflict, "encryption-required"), await RefusalAsync(await SendPlainAsync(maja, conversationId, "jawnie")));
        Assert.Equal(
            (HttpStatusCode.Conflict, "epoch-not-current"),
            await RefusalAsync(await SendEncryptedAsync(maja, conversationId, Guid.NewGuid().ToString())));

        var first = await RotateAsync(maja, conversationId);
        var ciphertext = Bytes(120);
        var sent = await SendEncryptedAsync(maja, conversationId, first, ciphertext: ciphertext);
        Assert.Equal(HttpStatusCode.Created, sent.StatusCode);
        var message = await JsonAsync(sent);
        Assert.Equal("", message.GetProperty("content").GetString());
        Assert.Equal(first, message.GetProperty("encrypted").GetProperty("epochId").GetString());
        Assert.Equal(ciphertext, message.GetProperty("encrypted").GetProperty("ciphertext").GetBytesFromBase64());
        Assert.Equal(12, message.GetProperty("encrypted").GetProperty("iv").GetBytesFromBase64().Length);

        var second = await RotateAsync(ada, conversationId);
        var outdated = await SendEncryptedAsync(maja, conversationId, first);
        Assert.Equal(HttpStatusCode.Conflict, outdated.StatusCode);
        var problem = await JsonAsync(outdated);
        Assert.Equal("epoch-not-current", problem.GetProperty("code").GetString());
        Assert.Equal(second, problem.GetProperty("currentEpochId").GetString());

        var withText = await maja.PostAsJsonAsync($"/api/social/conversations/{conversationId}/messages", new
        {
            content = "jawny dopisek",
            clientMessageId = Guid.NewGuid(),
            encrypted = new { epochId = second, iv = Bytes(12), ciphertext = Bytes(40) },
        });
        Assert.Equal(HttpStatusCode.BadRequest, withText.StatusCode);
        var shortIv = await maja.PostAsJsonAsync($"/api/social/conversations/{conversationId}/messages", new
        {
            clientMessageId = Guid.NewGuid(),
            encrypted = new { epochId = second, iv = Bytes(16), ciphertext = Bytes(40) },
        });
        Assert.Equal(HttpStatusCode.BadRequest, shortIv.StatusCode);

        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(ada, conversationId, "disable")).StatusCode);
        Assert.Equal((HttpStatusCode.Conflict, "encryption-off"), await RefusalAsync(await SendEncryptedAsync(maja, conversationId, second)));
        Assert.Equal(HttpStatusCode.Created, (await SendPlainAsync(maja, conversationId, "znowu jawnie")).StatusCode);
    }

    [Fact]
    public async Task AChangedSetOfDevicesAsksForANewEpochBeforeTheNextMessage()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        var conversationId = await OpenAsync(maja, "ada");
        await EnableAsync(maja, ada, conversationId);
        var first = await RotateAsync(maja, conversationId);
        Assert.Equal(HttpStatusCode.Created, (await SendEncryptedAsync(maja, conversationId, first)).StatusCode);

        using var adaPhone = await SignInAsync(factory, "ada");
        var phone = await RegisterDeviceAsync(adaPhone);
        var added = await SendEncryptedAsync(maja, conversationId, first);
        Assert.Equal(HttpStatusCode.Conflict, added.StatusCode);
        var problem = await JsonAsync(added);
        Assert.Equal("epoch-outdated", problem.GetProperty("code").GetString());
        Assert.Equal([phone], problem.GetProperty("missing").EnumerateArray().Select(x => x.GetString()));

        var second = await RotateAsync(maja, conversationId);
        Assert.Equal(HttpStatusCode.Created, (await SendEncryptedAsync(adaPhone, conversationId, second)).StatusCode);

        // Signing out takes the device out of the conversation without a line.
        Assert.Equal(HttpStatusCode.NoContent, (await adaPhone.PostAsync("/api/auth/logout", null)).StatusCode);
        var removed = await SendEncryptedAsync(maja, conversationId, second);
        problem = await JsonAsync(removed);
        Assert.Equal("epoch-outdated", problem.GetProperty("code").GetString());
        Assert.Equal([phone], problem.GetProperty("extra").EnumerateArray().Select(x => x.GetString()));
        Assert.Equal(["encryption_requested:maja", "encryption_enabled:ada", "device_added:ada"], await TimelineAsync(maja, conversationId));
    }

    [Fact]
    public async Task ARetriedEncryptedSendIsAnsweredWithTheStoredMessageEvenAfterTheKeysMovedOn()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        var conversationId = await OpenAsync(maja, "ada");
        await EnableAsync(maja, ada, conversationId);
        var first = await RotateAsync(maja, conversationId);
        var messageId = Guid.NewGuid();

        Assert.Equal(HttpStatusCode.Created, (await SendEncryptedAsync(maja, conversationId, first, messageId)).StatusCode);
        // Every encrypted message has empty content; distinct ones are not taken for retries.
        Assert.Equal(HttpStatusCode.Created, (await SendEncryptedAsync(maja, conversationId, first)).StatusCode);
        var retried = await SendEncryptedAsync(maja, conversationId, first, messageId);
        Assert.Equal(HttpStatusCode.OK, retried.StatusCode);
        Assert.Equal(messageId.ToString(), (await JsonAsync(retried)).GetProperty("id").GetString());

        var second = await RotateAsync(ada, conversationId);
        Assert.Equal(HttpStatusCode.OK, (await SendEncryptedAsync(maja, conversationId, second, messageId)).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(ada, conversationId, "disable")).StatusCode);
        Assert.Equal(HttpStatusCode.OK, (await SendEncryptedAsync(maja, conversationId, second, messageId)).StatusCode);

        Assert.Equal(HttpStatusCode.Conflict, (await SendPlainAsync(maja, conversationId, "jawnie", clientMessageId: messageId)).StatusCode);
        Assert.Equal(HttpStatusCode.Conflict, (await SendEncryptedAsync(ada, conversationId, second, messageId)).StatusCode);
        var withoutId = await maja.PostAsJsonAsync($"/api/social/conversations/{conversationId}/messages", new
        {
            encrypted = new { epochId = second, iv = Bytes(12), ciphertext = Bytes(40) },
        });
        Assert.Equal(HttpStatusCode.BadRequest, withoutId.StatusCode);

        var stored = (await HistoryAsync(ada, conversationId)).Where(x => x.GetProperty("system").ValueKind == JsonValueKind.Null).ToArray();
        Assert.Equal(2, stored.Length);
        Assert.Contains(stored, x => x.GetProperty("id").GetString() == messageId.ToString());
    }

    [Fact]
    public async Task EncryptedUploadsGoOnlyIntoEncryptedMessagesAndPlainOnesOnlyIntoPlainOnes()
    {
        using var factory = new StellarrWebApplicationFactory(
            loadTestData: true,
            new Dictionary<string, string?>
            {
                ["Uploads:MaxImageBytes"] = "1000",
                ["Uploads:MaxAudioBytes"] = "1000",
                ["Uploads:MaxVideoBytes"] = "2000",
                ["Uploads:MaxFileBytes"] = "1000",
            });
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        using var ilya = await SignInAsync(factory, "ilya");
        var conversationId = await OpenAsync(maja, "ada");

        // Bytes that are no known format are taken as they are, up to the
        // largest upload plus the authentication tag.
        var bytes = Bytes(2016);
        var encrypted = await UploadEncryptedAsync(maja, bytes);
        var plain = await UploadPngAsync(maja);
        using (var form = new MultipartFormDataContent())
        {
            form.Add(new ByteArrayContent(Bytes(2017)), "file", "blob");
            Assert.Equal(HttpStatusCode.RequestEntityTooLarge, (await maja.PostAsync("/api/uploads/encrypted", form)).StatusCode);
        }

        using (var form = new MultipartFormDataContent())
        {
            form.Add(new ByteArrayContent([]), "file", "blob");
            Assert.Equal(HttpStatusCode.BadRequest, (await maja.PostAsync("/api/uploads/encrypted", form)).StatusCode);
        }

        Assert.Equal(
            (HttpStatusCode.Conflict, "attachment-kind-mismatch"),
            await RefusalAsync(await SendPlainAsync(maja, conversationId, "z plikiem", [encrypted])));
        var channelMessage = await maja.PostAsJsonAsync(
            $"/api/channels/{TestData.GeneralChannelId}/messages",
            new { content = "na kanał", attachmentIds = new[] { encrypted } });
        Assert.Equal(HttpStatusCode.BadRequest, channelMessage.StatusCode);

        await EnableAsync(maja, ada, conversationId);
        var epochId = await RotateAsync(maja, conversationId);
        Assert.Equal(
            (HttpStatusCode.Conflict, "attachment-kind-mismatch"),
            await RefusalAsync(await SendEncryptedAsync(maja, conversationId, epochId, attachmentIds: [plain])));
        var sent = await SendEncryptedAsync(maja, conversationId, epochId, attachmentIds: [encrypted]);
        Assert.Equal(HttpStatusCode.Created, sent.StatusCode);
        var attachment = Assert.Single((await JsonAsync(sent)).GetProperty("attachments").EnumerateArray());
        Assert.Equal("application/octet-stream", attachment.GetProperty("contentType").GetString());
        Assert.Equal(2016, attachment.GetProperty("size").GetInt64());
        Assert.Equal(JsonValueKind.Null, attachment.GetProperty("width").ValueKind);
        Assert.Equal(JsonValueKind.Null, attachment.GetProperty("posterUrl").ValueKind);

        var download = await ada.GetAsync($"/assets/{encrypted}");
        Assert.Equal(HttpStatusCode.OK, download.StatusCode);
        Assert.Equal(bytes, await download.Content.ReadAsByteArrayAsync());
        Assert.Equal("attachment", download.Content.Headers.ContentDisposition?.DispositionType);
        Assert.Equal(HttpStatusCode.NotFound, (await ilya.GetAsync($"/assets/{encrypted}")).StatusCode);
    }

    [Fact]
    public async Task TheConversationListAndTheHistoryCarryTheCiphertextAndTheTimeline()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        var conversationId = await OpenAsync(maja, "ada");

        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(maja, conversationId, "request")).StatusCode);
        // A request waits for the other person's answer, so it counts as unread for them.
        var waiting = Assert.Single((await ada.GetFromJsonAsync<JsonElement>("/api/social")).GetProperty("conversations").EnumerateArray());
        Assert.Equal(1, waiting.GetProperty("unread").GetInt32());
        Assert.Equal("encryption_requested", waiting.GetProperty("lastMessage").GetProperty("system").GetProperty("kind").GetString());
        Assert.Equal("requested", waiting.GetProperty("encryption").GetProperty("state").GetString());

        Assert.Equal(HttpStatusCode.OK, (await SwitchAsync(ada, conversationId, "confirm")).StatusCode);
        var epochId = await RotateAsync(maja, conversationId);
        var ciphertext = Bytes(64);
        Assert.Equal(HttpStatusCode.Created, (await SendEncryptedAsync(maja, conversationId, epochId, ciphertext: ciphertext)).StatusCode);

        var listed = Assert.Single((await ada.GetFromJsonAsync<JsonElement>("/api/social")).GetProperty("conversations").EnumerateArray());
        var last = listed.GetProperty("lastMessage");
        Assert.Equal("", last.GetProperty("content").GetString());
        Assert.Equal(epochId, last.GetProperty("encrypted").GetProperty("epochId").GetString());
        Assert.Equal(ciphertext, last.GetProperty("encrypted").GetProperty("ciphertext").GetBytesFromBase64());
        Assert.Equal(JsonValueKind.Null, last.GetProperty("system").ValueKind);
        Assert.Equal(epochId, listed.GetProperty("encryption").GetProperty("currentEpochId").GetString());
        Assert.Equal(2, listed.GetProperty("unread").GetInt32());

        var history = await HistoryAsync(ada, conversationId);
        Assert.Equal(3, history.Length);
        Assert.Equal("encryption_requested", history[0].GetProperty("system").GetProperty("kind").GetString());
        Assert.Equal(People["maja"].Id.ToString(), history[0].GetProperty("system").GetProperty("actorId").GetString());
        Assert.Equal(People["maja"].Id.ToString(), history[0].GetProperty("author").GetProperty("id").GetString());
        Assert.Equal("encryption_enabled", history[1].GetProperty("system").GetProperty("kind").GetString());
        Assert.Equal(ciphertext, history[2].GetProperty("encrypted").GetProperty("ciphertext").GetBytesFromBase64());
    }

    [Fact]
    public async Task BothPeopleHearAboutSwitchesEpochsAndNewDevices()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        var conversationId = await OpenAsync(maja, "ada");
        await using var listener = await ListenAsync(factory, "ada");
        var changes = new List<JsonElement>();
        var lines = new List<JsonElement>();
        var epochs = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        var keysChanged = new TaskCompletionSource<JsonElement>(TaskCreationOptions.RunContinuationsAsynchronously);
        var enabled = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        using var changeSubscription = listener.On<JsonElement>("DirectEncryptionChanged", payload =>
        {
            lock (changes)
            {
                changes.Add(payload);
                if (payload.GetProperty("encryption").GetProperty("state").GetString() == "on")
                {
                    enabled.TrySetResult();
                }
            }
        });
        using var lineSubscription = listener.On<JsonElement>("DirectMessageCreated", payload =>
        {
            lock (lines)
            {
                lines.Add(payload);
            }
        });
        using var epochSubscription = listener.On<JsonElement>("DirectEncryptionEpochCreated", payload => epochs.TrySetResult(payload));
        using var keysSubscription = listener.On<JsonElement>("DirectEncryptionKeysChanged", payload => keysChanged.TrySetResult(payload));
        await listener.StartAsync();

        await EnableAsync(maja, ada, conversationId);
        await enabled.Task.WaitAsync(TimeSpan.FromSeconds(5));
        var keys = await KeysAsync(ada, conversationId);
        await RotateAsync(maja, conversationId);
        var epoch = await epochs.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.Equal(conversationId, epoch.GetProperty("conversationId").GetString());
        Assert.Equal(4, epoch.GetProperty("epoch").GetProperty("recipients").GetArrayLength());
        Assert.Equal(
            new[]
            {
                keys.GetProperty("self").GetProperty("accountKey").GetProperty("keyId").GetString(),
                keys.GetProperty("currentDeviceKeyId").GetString(),
            }.Order(),
            epoch.GetProperty("epoch").GetProperty("envelopes").EnumerateArray().Select(x => x.GetProperty("recipientKeyId").GetString()).Order());

        using var majaPhone = await SignInAsync(factory, "maja");
        await RegisterDeviceAsync(majaPhone);
        var keyChange = await keysChanged.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.Equal(People["maja"].Id.ToString(), keyChange.GetProperty("userId").GetString());

        lock (changes)
        {
            Assert.Equal(["requested", "on"], changes.Select(x => x.GetProperty("encryption").GetProperty("state").GetString()));
        }

        lock (lines)
        {
            Assert.Equal(
                ["encryption_requested", "encryption_enabled", "device_added"],
                lines.Select(x => x.GetProperty("system").GetProperty("kind").GetString()));
        }
    }

    [Fact]
    public async Task ANewSignInIsNotAnnouncedAcrossABlockButStaysInTheHistory()
    {
        using var factory = new StellarrWebApplicationFactory();
        using var maja = await ReadyAsync(factory, "maja");
        using var ada = await ReadyAsync(factory, "ada");
        var conversationId = await OpenAsync(maja, "ada");
        await EnableAsync(maja, ada, conversationId);
        await using var listener = await ListenAsync(factory, "ada");
        var leaked = new List<string>();
        var ownChange = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        using var lineSubscription = listener.On<JsonElement>("DirectMessageCreated", _ =>
        {
            lock (leaked)
            {
                leaked.Add("DirectMessageCreated");
            }
        });
        using var keysSubscription = listener.On<JsonElement>("DirectEncryptionKeysChanged", _ =>
        {
            lock (leaked)
            {
                leaked.Add("DirectEncryptionKeysChanged");
            }
        });
        using var accountSubscription = listener.On<JsonElement>("EncryptionAccountChanged", _ => ownChange.TrySetResult());
        await listener.StartAsync();
        (await ada.PostAsync($"/api/social/blocks/{People["maja"].Id}", null)).EnsureSuccessStatusCode();

        using var majaLaptop = await SignInAsync(factory, "maja");
        await RegisterDeviceAsync(majaLaptop);
        // Ada's own change reaches her after anything the laptop's sign-in sent.
        using var adaPhone = await SignInAsync(factory, "ada");
        await RegisterDeviceAsync(adaPhone);
        await ownChange.Task.WaitAsync(TimeSpan.FromSeconds(5));

        lock (leaked)
        {
            Assert.Empty(leaked);
        }

        (await ada.DeleteAsync($"/api/social/blocks/{People["maja"].Id}")).EnsureSuccessStatusCode();
        Assert.Equal(
            ["encryption_requested:maja", "encryption_enabled:ada", "device_added:maja", "device_added:ada"],
            await TimelineAsync(ada, conversationId));
    }
}
