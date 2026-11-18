using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.AspNetCore.Mvc.Testing;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Mail;
using Stellarr.Api.Notifications;

namespace Stellarr.Api.Tests;

public sealed class NotificationDeliveryTests
{
    private static readonly Guid MajaId = Guid.Parse("30000000-0000-0000-0000-000000000001");
    private static readonly Guid AdaId = Guid.Parse("30000000-0000-0000-0000-000000000002");
    private static readonly Guid IlyaId = Guid.Parse("30000000-0000-0000-0000-000000000003");
    private static readonly Guid NilsId = Guid.Parse("30000000-0000-0000-0000-000000000004");

    [Fact]
    public async Task MentionsEnqueueOneRowPerOptedInDestinationAndSendGenericContent()
    {
        var push = new FakeWebPushDelivery();
        using var factory = NewFactory(push);
        var subscriptionId = await AddPushSubscriptionAsync(factory, AdaId, "https://push.example.test/ada");
        await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        await SetPreferencesAsync(factory, AdaId, email: true, push: true);
        // Ilya opted in too, but has no verified address and no subscription.
        await SetPreferencesAsync(factory, IlyaId, email: true, push: true);
        // Nils never opted in, so no row is written for him at all.
        await SetPreferencesAsync(factory, NilsId, email: false, push: false);
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");

        var messageId = Guid.NewGuid();
        await SendMentionAsync(maja, messageId, "Hej @Ada, witaj też @Ilya i @Nils");

        var rows = await DeliveryRowsAsync(factory, messageId);
        Assert.Equal(2, rows.Length);
        Assert.All(rows, row => Assert.Equal(AdaId, row.RecipientId));
        Assert.DoesNotContain(rows, row => row.RecipientId == IlyaId || row.RecipientId == NilsId);
        Assert.All(rows, row => Assert.Equal(NotificationDeliveryKinds.Mention, row.EventKind));
        var emailRow = Assert.Single(rows, row => row.Channel == NotificationDeliveryChannels.Email);
        Assert.Equal("ADA@EXAMPLE.TEST", emailRow.DestinationKey);
        Assert.Null(emailRow.PushSubscriptionId);
        var pushRow = Assert.Single(rows, row => row.Channel == NotificationDeliveryChannels.Push);
        Assert.Equal(subscriptionId, pushRow.PushSubscriptionId);
        Assert.Equal(subscriptionId.ToString("N"), pushRow.DestinationKey);

        var delivered = await DispatchAsync(factory);

        Assert.Equal(2, delivered);
        var mail = Assert.Single(factory.EmailDelivery.Messages);
        Assert.Equal("ada@example.test", mail.To);
        Assert.Equal("New mention on Stellarr", mail.Subject);
        Assert.Contains("Someone mentioned you", mail.Text, StringComparison.Ordinal);
        Assert.Contains("https://chat.test/", mail.Text, StringComparison.Ordinal);
        Assert.DoesNotContain("Maja", mail.Text, StringComparison.Ordinal);
        Assert.DoesNotContain("Orbit", mail.Text, StringComparison.Ordinal);
        Assert.DoesNotContain("ogólny", mail.Text, StringComparison.Ordinal);
        Assert.DoesNotContain("Hej", mail.Text, StringComparison.Ordinal);
        Assert.StartsWith($"activity-notification/{emailRow.Id:N}", mail.IdempotencyKey, StringComparison.Ordinal);

        var sentPush = Assert.Single(push.Sent);
        Assert.Equal(subscriptionId, sentPush.SubscriptionId);
        Assert.Equal("New mention", sentPush.Payload.Title);
        Assert.Equal("https://chat.test/", sentPush.Payload.Url);
        Assert.Contains("Someone mentioned you", sentPush.Payload.Body, StringComparison.Ordinal);
        Assert.DoesNotContain("Maja", sentPush.Payload.Body, StringComparison.Ordinal);
        Assert.DoesNotContain("Orbit", sentPush.Payload.Body, StringComparison.Ordinal);
        Assert.DoesNotContain("ogólny", sentPush.Payload.Body, StringComparison.Ordinal);
        Assert.DoesNotContain("Hej", sentPush.Payload.Body, StringComparison.Ordinal);

        rows = await DeliveryRowsAsync(factory, messageId);
        Assert.All(rows, row =>
        {
            Assert.Equal(NotificationDeliveryStatuses.Sent, row.Status);
            Assert.NotNull(row.SentAt);
        });
    }

    [Fact]
    public async Task DirectMessagesEnqueueOnlyThePeerAndReplayAddsNoJobs()
    {
        var push = new FakeWebPushDelivery();
        using var factory = NewFactory(push);
        var subscriptionId = await AddPushSubscriptionAsync(factory, AdaId, "https://push.example.test/ada");
        await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        await SetPreferencesAsync(factory, AdaId, email: true, push: true);
        // The sender's own choices must not make the sender a recipient.
        await SetPreferencesAsync(factory, MajaId, email: true, push: true);
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        var conversationId = await OpenConversationAsync(maja, AdaId);

        var messageId = Guid.NewGuid();
        await SendDirectMessageAsync(maja, conversationId, messageId, "sekret");
        var replay = await SendDirectMessageAsync(maja, conversationId, messageId, "sekret");

        Assert.Equal(HttpStatusCode.OK, replay.StatusCode);
        var rows = await DeliveryRowsAsync(factory, messageId);
        Assert.Equal(2, rows.Length);
        Assert.All(rows, row =>
        {
            Assert.Equal(AdaId, row.RecipientId);
            Assert.Equal(NotificationDeliveryKinds.DirectMessage, row.EventKind);
        });
        Assert.Contains(rows, row =>
            row.Channel == NotificationDeliveryChannels.Push && row.PushSubscriptionId == subscriptionId);

        Assert.Equal(2, await DispatchAsync(factory));
        var mail = Assert.Single(factory.EmailDelivery.Messages);
        Assert.Equal("New direct message on Stellarr", mail.Subject);
        Assert.Contains("You have a new direct message", mail.Text, StringComparison.Ordinal);
        Assert.DoesNotContain("sekret", mail.Text, StringComparison.Ordinal);
        Assert.DoesNotContain("Maja", mail.Text, StringComparison.Ordinal);
        var sentPush = Assert.Single(push.Sent);
        Assert.StartsWith("New direct message", sentPush.Payload.Title, StringComparison.Ordinal);
        Assert.DoesNotContain("sekret", sentPush.Payload.Body, StringComparison.Ordinal);
    }

    [Fact]
    public async Task AnEncryptedDirectMessageIsAnnouncedWithNothingOfItsTextOrEnvelope()
    {
        var push = new FakeWebPushDelivery();
        using var factory = NewFactory(push);
        // Ada signs in before her address becomes a second step she would be asked for.
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        using var ada = await LoginAsync(factory, "ada", "Quasar!2026");
        await AddPushSubscriptionAsync(factory, AdaId, "https://push.example.test/ada");
        await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        await SetPreferencesAsync(factory, AdaId, email: true, push: true);
        foreach (var client in new[] { maja, ada })
        {
            await EncryptionTestKit.SetUpAsync(client);
            await EncryptionTestKit.RegisterDeviceAsync(client);
        }

        var conversationId = (await OpenConversationAsync(maja, AdaId)).ToString();
        await EncryptionTestKit.EnableAsync(maja, ada, conversationId);
        var epochId = await EncryptionTestKit.RotateAsync(maja, conversationId);
        var uploadId = await EncryptionTestKit.UploadEncryptedAsync(maja);
        var messageId = Guid.NewGuid();
        var iv = EncryptionTestKit.Bytes(12);
        var ciphertext = EncryptionTestKit.Bytes(96);
        var sent = await maja.PostAsJsonAsync($"/api/social/conversations/{conversationId}/messages", new
        {
            clientMessageId = messageId,
            attachmentIds = new[] { uploadId },
            encrypted = new { epochId, iv, ciphertext },
        });
        Assert.Equal(HttpStatusCode.Created, sent.StatusCode);
        Assert.Equal(2, (await DeliveryRowsAsync(factory, messageId)).Length);

        Assert.Equal(2, await DispatchAsync(factory));

        // Nothing the message carried may leave in a notice: not the bytes in
        // either spelling, not the epoch, the upload or the sender.
        var secrets = new[]
        {
            Convert.ToBase64String(ciphertext), Convert.ToHexString(ciphertext),
            Convert.ToBase64String(iv), Convert.ToHexString(iv),
            epochId, uploadId, conversationId, "Maja",
        };
        var mail = Assert.Single(factory.EmailDelivery.Messages);
        Assert.Equal("New direct message on Stellarr", mail.Subject);
        Assert.Contains("You have a new direct message", mail.Text, StringComparison.Ordinal);
        var sentPush = Assert.Single(push.Sent);
        Assert.Equal("New direct message", sentPush.Payload.Title);
        Assert.Equal("You have a new direct message. Open Stellarr to read it.", sentPush.Payload.Body);
        var pushed = string.Join(
            ' ', sentPush.Payload.Id, sentPush.Payload.Title, sentPush.Payload.Body, sentPush.Payload.Url, sentPush.Payload.Tag);
        foreach (var secret in secrets)
        {
            Assert.DoesNotContain(secret, mail.Subject, StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain(secret, mail.Text, StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain(secret, mail.Html, StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain(secret, pushed, StringComparison.OrdinalIgnoreCase);
        }
    }

    [Fact]
    public async Task ConcurrentAndSequentialClientMessageReplaysCreateOneSetOfJobs()
    {
        using var factory = NewFactory(new FakeWebPushDelivery());
        await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        await SetPreferencesAsync(factory, AdaId, email: true);
        using var first = await LoginAsync(factory, "maja", "Aurora!2026");
        using var second = await LoginAsync(factory, "maja", "Aurora!2026");

        var channelMessageId = Guid.NewGuid();
        var responses = await Task.WhenAll(
            SendMentionAsync(first, channelMessageId, "@Ada jedno"),
            SendMentionAsync(second, channelMessageId, "@Ada jedno"));
        Assert.DoesNotContain(responses, response => (int)response.StatusCode >= 500);
        Assert.Single(await DeliveryRowsAsync(factory, channelMessageId));

        var conversationId = await OpenConversationAsync(first, AdaId);
        var firstSend = await SendDirectMessageAsync(first, conversationId, Guid.NewGuid(), "raz");
        Assert.Equal(HttpStatusCode.Created, firstSend.StatusCode);
        var messageId = await ReadIdAsync(firstSend);
        await SendDirectMessageAsync(first, conversationId, messageId, "raz");
        Assert.Single(await DeliveryRowsAsync(factory, messageId));
    }

    [Theory]
    [InlineData("membership")]
    [InlineData("message")]
    [InlineData("channel")]
    [InlineData("mute")]
    [InlineData("read")]
    [InlineData("edit")]
    [InlineData("disabled")]
    [InlineData("blocked")]
    [InlineData("preference")]
    public async Task AChangedMentionIsCancelledWithoutCallingAProvider(string reason)
    {
        var push = new FakeWebPushDelivery();
        using var factory = NewFactory(push);
        await AddPushSubscriptionAsync(factory, AdaId, "https://push.example.test/ada");
        await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        await SetPreferencesAsync(factory, AdaId, email: true, push: true);
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        var messageId = Guid.NewGuid();
        await SendMentionAsync(maja, messageId, "@Ada zobacz");

        await using (var scope = factory.OpenScope(out var db))
        {
            switch (reason)
            {
                case "membership":
                    await db.ServerMembers
                        .Where(x => x.ServerId == TestData.ServerId && x.UserId == AdaId)
                        .ExecuteDeleteAsync();
                    break;
                case "message":
                    await db.Messages.Where(x => x.Id == messageId).ExecuteDeleteAsync();
                    break;
                case "channel":
                    await db.Channels.Where(x => x.Id == TestData.GeneralChannelId).ExecuteDeleteAsync();
                    break;
                case "mute":
                    var muted = await db.UserPreferences.SingleAsync(x => x.UserId == AdaId);
                    muted.MutedChannelIdsJson = JsonSerializer.Serialize(new[] { TestData.GeneralChannelId.ToString() });
                    await db.SaveChangesAsync();
                    break;
                case "read":
                    db.ChannelReads.Add(new ChannelRead
                    {
                        ChannelId = TestData.GeneralChannelId,
                        UserId = AdaId,
                        LastReadAt = DateTime.UtcNow.AddMinutes(1),
                    });
                    await db.SaveChangesAsync();
                    break;
                case "edit":
                    var message = await db.Messages.SingleAsync(x => x.Id == messageId);
                    message.Content = "bez wzmianki";
                    await db.SaveChangesAsync();
                    break;
                case "disabled":
                    var user = await db.Users.SingleAsync(x => x.Id == AdaId);
                    user.IsDisabled = true;
                    await db.SaveChangesAsync();
                    break;
                case "blocked":
                    db.UserBlocks.Add(new UserBlock
                    {
                        BlockerId = AdaId,
                        BlockedId = MajaId,
                        CreatedAt = DateTime.UtcNow,
                    });
                    await db.SaveChangesAsync();
                    break;
                case "preference":
                    var preferences = await db.UserPreferences.SingleAsync(x => x.UserId == AdaId);
                    preferences.EmailNotifications = false;
                    preferences.PushNotifications = false;
                    await db.SaveChangesAsync();
                    break;
                default:
                    throw new ArgumentOutOfRangeException(nameof(reason), reason, null);
            }
        }

        Assert.Equal(0, await DispatchAsync(factory));
        Assert.Empty(factory.EmailDelivery.Messages);
        Assert.Empty(push.Sent);
        var rows = await DeliveryRowsAsync(factory, messageId);
        Assert.NotEmpty(rows);
        Assert.All(rows, row => Assert.Equal(NotificationDeliveryStatuses.Cancelled, row.Status));
    }

    [Theory]
    [InlineData("blocked")]
    [InlineData("disabled")]
    [InlineData("message")]
    [InlineData("allow-direct-messages")]
    [InlineData("read")]
    [InlineData("preference")]
    public async Task AChangedDirectMessageIsCancelledWithoutCallingAProvider(string reason)
    {
        var push = new FakeWebPushDelivery();
        using var factory = NewFactory(push);
        await AddPushSubscriptionAsync(factory, AdaId, "https://push.example.test/ada");
        await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        await SetPreferencesAsync(factory, AdaId, email: true, push: true);
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        var conversationId = await OpenConversationAsync(maja, AdaId);
        var messageId = Guid.NewGuid();
        await SendDirectMessageAsync(maja, conversationId, messageId, "wiadomość");

        await using (var scope = factory.OpenScope(out var db))
        {
            switch (reason)
            {
                case "blocked":
                    db.UserBlocks.Add(new UserBlock
                    {
                        BlockerId = MajaId,
                        BlockedId = AdaId,
                        CreatedAt = DateTime.UtcNow,
                    });
                    await db.SaveChangesAsync();
                    break;
                case "disabled":
                    var user = await db.Users.SingleAsync(x => x.Id == AdaId);
                    user.IsDisabled = true;
                    await db.SaveChangesAsync();
                    break;
                case "message":
                    await db.DirectMessages.Where(x => x.Id == messageId).ExecuteDeleteAsync();
                    break;
                case "allow-direct-messages":
                    var preferences = await db.UserPreferences.SingleAsync(x => x.UserId == AdaId);
                    preferences.AllowDirectMessages = false;
                    await db.SaveChangesAsync();
                    break;
                case "read":
                    db.DirectConversationReads.Add(new DirectConversationRead
                    {
                        ConversationId = conversationId,
                        UserId = AdaId,
                        LastReadAt = DateTime.UtcNow.AddMinutes(1),
                    });
                    await db.SaveChangesAsync();
                    break;
                case "preference":
                    var notices = await db.UserPreferences.SingleAsync(x => x.UserId == AdaId);
                    notices.EmailNotifications = false;
                    notices.PushNotifications = false;
                    await db.SaveChangesAsync();
                    break;
                default:
                    throw new ArgumentOutOfRangeException(nameof(reason), reason, null);
            }
        }

        Assert.Equal(0, await DispatchAsync(factory));
        Assert.Empty(factory.EmailDelivery.Messages);
        Assert.Empty(push.Sent);
        var rows = await DeliveryRowsAsync(factory, messageId);
        Assert.NotEmpty(rows);
        Assert.All(rows, row => Assert.Equal(NotificationDeliveryStatuses.Cancelled, row.Status));
    }

    [Fact]
    public async Task ARefusedPushDestinationRetriesAloneWithoutResendingTheOthers()
    {
        var push = new FakeWebPushDelivery();
        using var factory = NewFactory(push);
        var workingId = await AddPushSubscriptionAsync(factory, AdaId, "https://push.example.test/working");
        var failingId = await AddPushSubscriptionAsync(factory, AdaId, "https://push.example.test/failing");
        push.ResultFor = subscription =>
            subscription.Id == failingId ? PushDeliveryResult.Retry : PushDeliveryResult.Delivered;
        await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        await SetPreferencesAsync(factory, AdaId, email: true, push: true);
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        var messageId = Guid.NewGuid();
        await SendMentionAsync(maja, messageId, "@Ada");

        Assert.Equal(2, await DispatchAsync(factory));
        var mail = Assert.Single(factory.EmailDelivery.Messages);
        Assert.Single(push.Sent);
        Assert.Equal(workingId, push.Sent[0].SubscriptionId);
        Assert.Equal(2, push.Calls);

        var rows = await DeliveryRowsAsync(factory, messageId);
        var pending = Assert.Single(rows, row => row.Status == NotificationDeliveryStatuses.Pending);
        Assert.Equal(failingId, pending.PushSubscriptionId);
        Assert.Equal(1, pending.Attempts);
        Assert.True(pending.NextAttemptAt > DateTime.UtcNow);

        push.ResultFor = _ => PushDeliveryResult.Delivered;
        Assert.Equal(1, await DispatchAsync(factory, pending.NextAttemptAt.AddSeconds(1)));

        Assert.Single(factory.EmailDelivery.Messages);
        Assert.Equal(2, push.Sent.Count);
        Assert.Equal(3, push.Calls);
        Assert.All(await DeliveryRowsAsync(factory, messageId), row =>
            Assert.Equal(NotificationDeliveryStatuses.Sent, row.Status));
        Assert.NotNull(mail);
    }

    [Fact]
    public async Task AMailFailureRetriesOnlyTheMailWithTheSameIdempotencyKey()
    {
        var push = new FakeWebPushDelivery();
        using var factory = NewFactory(push);
        await AddPushSubscriptionAsync(factory, AdaId, "https://push.example.test/ada");
        await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        await SetPreferencesAsync(factory, AdaId, email: true, push: true);
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        var messageId = Guid.NewGuid();
        await SendMentionAsync(maja, messageId, "@Ada");
        factory.EmailDelivery.Failure = new MailDeliveryException("provider refused");

        Assert.Equal(1, await DispatchAsync(factory));
        Assert.Empty(factory.EmailDelivery.Messages);
        Assert.Single(push.Sent);
        var failedMail = Assert.Single(
            await DeliveryRowsAsync(factory, messageId),
            row => row.Channel == NotificationDeliveryChannels.Email);
        Assert.Equal(NotificationDeliveryStatuses.Pending, failedMail.Status);
        Assert.Equal(1, failedMail.Attempts);
        Assert.Equal("delivery failed", failedMail.LastError);

        factory.EmailDelivery.Failure = null;
        Assert.Equal(1, await DispatchAsync(factory, failedMail.NextAttemptAt.AddSeconds(1)));

        var mail = Assert.Single(factory.EmailDelivery.Messages);
        Assert.Equal($"activity-notification/{failedMail.Id:N}", mail.IdempotencyKey);
        Assert.Single(push.Sent);
        Assert.Equal(1, push.Calls);
        Assert.All(await DeliveryRowsAsync(factory, messageId), row =>
            Assert.Equal(NotificationDeliveryStatuses.Sent, row.Status));
    }

    [Fact]
    public async Task AGoneSubscriptionIsRemovedAndItsDeliveryIsCompleted()
    {
        var push = new FakeWebPushDelivery { ResultFor = _ => PushDeliveryResult.Gone };
        using var factory = NewFactory(push);
        var subscriptionId = await AddPushSubscriptionAsync(factory, AdaId, "https://push.example.test/gone");
        await SetPreferencesAsync(factory, AdaId, push: true);
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        var messageId = Guid.NewGuid();
        await SendMentionAsync(maja, messageId, "@Ada");

        Assert.Equal(0, await DispatchAsync(factory));
        Assert.Empty(push.Sent);
        Assert.Equal(1, push.Calls);
        var rows = await DeliveryRowsAsync(factory, messageId);
        Assert.All(rows, row => Assert.Equal(NotificationDeliveryStatuses.Cancelled, row.Status));
        await using (var scope = factory.OpenScope(out var db))
        {
            Assert.False(await db.PushSubscriptions.AnyAsync(x => x.Id == subscriptionId));
        }

        // A later message has no destination left.
        await SendMentionAsync(maja, Guid.NewGuid(), "@Ada ponownie");
        await using var verification = factory.OpenScope(out var verificationDb);
        Assert.Equal(1, await verificationDb.NotificationDeliveries.CountAsync());
    }

    [Fact]
    public async Task ExpiredRowsAreCancelledAndOldCompletedRowsArePruned()
    {
        using var factory = NewFactory(new FakeWebPushDelivery());
        await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        await SetPreferencesAsync(factory, AdaId, email: true);
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        var messageId = Guid.NewGuid();
        await SendMentionAsync(maja, messageId, "@Ada");
        var oldId = Guid.NewGuid();

        await using (var scope = factory.OpenScope(out var db))
        {
            var row = await db.NotificationDeliveries.SingleAsync(x => x.SourceMessageId == messageId);
            row.ExpiresAt = DateTime.UtcNow.AddMinutes(-1);
            row.NextAttemptAt = DateTime.UtcNow.AddMinutes(-1);
            db.NotificationDeliveries.Add(new NotificationDelivery
            {
                Id = oldId,
                SourceMessageId = Guid.NewGuid(),
                RecipientId = AdaId,
                EventKind = NotificationDeliveryKinds.Mention,
                Channel = NotificationDeliveryChannels.Email,
                DestinationKey = "ADA@EXAMPLE.TEST",
                CreatedAt = DateTime.UtcNow.AddDays(-40),
                ExpiresAt = DateTime.UtcNow.AddDays(-39),
                NextAttemptAt = DateTime.UtcNow.AddDays(-40),
                Status = NotificationDeliveryStatuses.Cancelled,
            });
            await db.SaveChangesAsync();
        }

        Assert.Equal(0, await DispatchAsync(factory));
        Assert.Empty(factory.EmailDelivery.Messages);
        await using var verification = factory.OpenScope(out var verificationDb);
        Assert.False(await verificationDb.NotificationDeliveries.AnyAsync(x => x.Id == oldId));
        Assert.Equal(
            NotificationDeliveryStatuses.Cancelled,
            (await verificationDb.NotificationDeliveries.SingleAsync(x => x.SourceMessageId == messageId)).Status);
    }

    [Fact]
    public async Task NoMailIsAttemptedOrDiscardedWhenMailingIsUnconfigured()
    {
        using var factory = NewFactory(
            new FakeWebPushDelivery(),
            new Dictionary<string, string?> { ["Mail:Enabled"] = "false" });
        await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        await SetPreferencesAsync(factory, AdaId, email: true);
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        var messageId = Guid.NewGuid();
        await SendMentionAsync(maja, messageId, "@Ada");

        var before = DateTime.UtcNow;
        Assert.Equal(0, await DispatchAsync(factory));

        Assert.Empty(factory.EmailDelivery.Messages);
        var row = Assert.Single(await DeliveryRowsAsync(factory, messageId));
        Assert.Equal(NotificationDeliveryStatuses.Pending, row.Status);
        Assert.Equal(0, row.Attempts);
        Assert.True(row.NextAttemptAt > before.AddMinutes(30));
    }

    [Theory]
    [InlineData("none")]
    [InlineData("changed")]
    [InlineData("revoked")]
    public async Task MailNeedsTheSameVerifiedAddressAtDeliveryTime(string state)
    {
        using var factory = NewFactory(new FakeWebPushDelivery());
        await SetPreferencesAsync(factory, AdaId, email: true);
        if (state != "none")
        {
            await VerifyEmailAsync(factory, AdaId, "ada@example.test");
        }

        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        var messageId = Guid.NewGuid();
        await SendMentionAsync(maja, messageId, "@Ada");
        if (state == "none")
        {
            Assert.Empty(await DeliveryRowsAsync(factory, messageId));
            Assert.Equal(0, await DispatchAsync(factory));
            Assert.Empty(factory.EmailDelivery.Messages);
            return;
        }

        await using (var scope = factory.OpenScope(out var db))
        {
            if (state == "changed")
            {
                var user = await db.Users.SingleAsync(x => x.Id == AdaId);
                user.Email = "new@example.test";
                user.NormalizedEmail = "NEW@EXAMPLE.TEST";
                db.MfaMethods.Add(new MfaMethod
                {
                    Id = Guid.NewGuid(),
                    UserId = AdaId,
                    Type = MfaMethodTypes.Email,
                    Name = "E-mail",
                    CreatedAt = DateTime.UtcNow,
                    ConfirmedAt = DateTime.UtcNow,
                    EmailAddress = "NEW@EXAMPLE.TEST",
                });
            }
            else
            {
                var method = await db.MfaMethods.SingleAsync(x =>
                    x.UserId == AdaId && x.Type == MfaMethodTypes.Email);
                method.RevokedAt = DateTime.UtcNow;
            }

            await db.SaveChangesAsync();
        }

        Assert.Equal(0, await DispatchAsync(factory));
        Assert.Empty(factory.EmailDelivery.Messages);
        var row = Assert.Single(await DeliveryRowsAsync(factory, messageId));
        Assert.Equal(NotificationDeliveryStatuses.Cancelled, row.Status);
    }

    [Theory]
    [InlineData("deleted")]
    [InlineData("revoked-session")]
    [InlineData("expired-session")]
    [InlineData("session-version")]
    [InlineData("new-owner")]
    public async Task PushNeedsALiveSubscriptionOwnedByTheRecipient(string state)
    {
        var push = new FakeWebPushDelivery();
        using var factory = NewFactory(push);
        var subscriptionId = await AddPushSubscriptionAsync(factory, AdaId, "https://push.example.test/ada");
        await SetPreferencesAsync(factory, AdaId, push: true);
        using var maja = await LoginAsync(factory, "maja", "Aurora!2026");
        var messageId = Guid.NewGuid();
        await SendMentionAsync(maja, messageId, "@Ada");

        await using (var scope = factory.OpenScope(out var db))
        {
            var subscription = await db.PushSubscriptions.SingleAsync(x => x.Id == subscriptionId);
            switch (state)
            {
                case "deleted":
                    db.PushSubscriptions.Remove(subscription);
                    break;
                case "revoked-session":
                    var revoked = await db.UserSessions.SingleAsync(x => x.Id == subscription.SessionId);
                    revoked.RevokedAt = DateTime.UtcNow;
                    break;
                case "expired-session":
                    var expired = await db.UserSessions.SingleAsync(x => x.Id == subscription.SessionId);
                    expired.ExpiresAt = DateTime.UtcNow.AddMinutes(-1);
                    expired.AbsoluteExpiresAt = DateTime.UtcNow.AddMinutes(-1);
                    break;
                case "session-version":
                    var user = await db.Users.SingleAsync(x => x.Id == AdaId);
                    user.SessionVersion++;
                    break;
                case "new-owner":
                    subscription.UserId = IlyaId;
                    break;
                default:
                    throw new ArgumentOutOfRangeException(nameof(state), state, null);
            }

            await db.SaveChangesAsync();
        }

        Assert.Equal(0, await DispatchAsync(factory));
        Assert.Empty(push.Sent);
        var row = Assert.Single(await DeliveryRowsAsync(factory, messageId));
        Assert.Equal(NotificationDeliveryStatuses.Cancelled, row.Status);
    }

    [Fact]
    public async Task PreparingTheSchemaTwiceRecreatesAMissingOutboxTable()
    {
        using var factory = NewFactory(new FakeWebPushDelivery());
        await using (var scope = factory.OpenScope(out var db))
        {
            await db.Database.ExecuteSqlRawAsync("DROP TABLE \"NotificationDeliveries\";");
            await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
            await DatabaseSchema.PrepareAsync(db, CancellationToken.None);
            Assert.Equal(0, await db.NotificationDeliveries.CountAsync());
            db.NotificationDeliveries.Add(new NotificationDelivery
            {
                Id = Guid.NewGuid(),
                SourceMessageId = Guid.NewGuid(),
                RecipientId = AdaId,
                EventKind = NotificationDeliveryKinds.Mention,
                Channel = NotificationDeliveryChannels.Email,
                DestinationKey = "ADA@EXAMPLE.TEST",
                CreatedAt = DateTime.UtcNow,
                ExpiresAt = DateTime.UtcNow.AddDays(1),
                NextAttemptAt = DateTime.UtcNow,
                Status = NotificationDeliveryStatuses.Pending,
            });
            await db.SaveChangesAsync();
            Assert.Equal(1, await db.NotificationDeliveries.CountAsync());
        }
    }

    [Fact]
    public void ActivityMailEscapesTheLinkAndNamesNobody()
    {
        var deliveryId = Guid.NewGuid();
        var mail = ActivityEmail.Mention(
            "Stellarr <noreply@chat.test>",
            "ada@example.test",
            "https://chat.test/app?a=1&b=2",
            deliveryId,
            new Messages(new LanguageOptions { Language = LanguageOptions.Polish }));

        Assert.Equal("Nowa wzmianka w Stellarr", mail.Subject);
        Assert.Contains("https://chat.test/app?a=1&amp;b=2/", mail.Html, StringComparison.Ordinal);
        Assert.Contains("https://chat.test/app?a=1&b=2/", mail.Text, StringComparison.Ordinal);
        Assert.Contains("Ktoś wspomniał o Tobie", mail.Text, StringComparison.Ordinal);
        Assert.DoesNotContain("Maja", mail.Text, StringComparison.Ordinal);
        Assert.Equal($"activity-notification/{deliveryId:N}", mail.IdempotencyKey);
    }

    private static StellarrWebApplicationFactory NewFactory(
        FakeWebPushDelivery push,
        IReadOnlyDictionary<string, string?>? configuration = null)
    {
        var settings = new Dictionary<string, string?>
        {
            ["Notifications:WorkerEnabled"] = "false",
            // The suite's module initializer makes every other host Polish.
            ["Stellarr:Language"] = LanguageOptions.English,
        };
        foreach (var (key, value) in configuration ?? new Dictionary<string, string?>())
        {
            settings[key] = value;
        }

        return new StellarrWebApplicationFactory(
            loadTestData: true,
            configuration: settings,
            configureServices: services =>
            {
                services.RemoveAll<IWebPushDelivery>();
                services.AddSingleton<IWebPushDelivery>(push);
            });
    }

    private static async Task<HttpClient> LoginAsync(
        StellarrWebApplicationFactory factory,
        string username,
        string password)
    {
        var client = factory.CreateClient(new WebApplicationFactoryClientOptions
        {
            AllowAutoRedirect = false,
            HandleCookies = true,
        });
        var login = await client.PostAsJsonAsync("/api/auth/login", new { username, password });
        login.EnsureSuccessStatusCode();
        return client;
    }

    private static async Task SetPreferencesAsync(
        StellarrWebApplicationFactory factory,
        Guid userId,
        bool email = false,
        bool push = false,
        bool allowDirectMessages = true,
        Guid[]? mutedChannels = null)
    {
        await using var scope = factory.OpenScope(out var db);
        var preferences = await db.UserPreferences.SingleOrDefaultAsync(x => x.UserId == userId);
        if (preferences is null)
        {
            preferences = new UserPreferenceSettings { UserId = userId };
            db.UserPreferences.Add(preferences);
        }

        preferences.EmailNotifications = email;
        preferences.PushNotifications = push;
        preferences.AllowDirectMessages = allowDirectMessages;
        if (mutedChannels is not null)
        {
            preferences.MutedChannelIdsJson = JsonSerializer.Serialize(mutedChannels);
        }

        await db.SaveChangesAsync();
    }

    private static async Task VerifyEmailAsync(
        StellarrWebApplicationFactory factory,
        Guid userId,
        string address)
    {
        await using var scope = factory.OpenScope(out var db);
        var user = await db.Users.SingleAsync(x => x.Id == userId);
        user.Email = address;
        user.NormalizedEmail = address.ToUpperInvariant();
        db.MfaMethods.Add(new MfaMethod
        {
            Id = Guid.NewGuid(),
            UserId = userId,
            Type = MfaMethodTypes.Email,
            Name = "E-mail",
            CreatedAt = DateTime.UtcNow,
            ConfirmedAt = DateTime.UtcNow,
            EmailAddress = address.ToUpperInvariant(),
        });
        await db.SaveChangesAsync();
    }

    private static async Task<Guid> AddPushSubscriptionAsync(
        StellarrWebApplicationFactory factory,
        Guid userId,
        string endpoint)
    {
        await using var scope = factory.OpenScope(out var db);
        var user = await db.Users.AsNoTracking().SingleAsync(x => x.Id == userId);
        var now = DateTime.UtcNow;
        var session = new UserSession
        {
            Id = Guid.NewGuid(),
            UserId = userId,
            Device = "test",
            IpAddress = "127.0.0.1",
            CreatedAt = now,
            LastSeenAt = now,
            ExpiresAt = now.AddDays(1),
            AbsoluteExpiresAt = now.AddDays(1),
            SecurityVersion = user.SessionVersion,
        };
        var subscription = new PushSubscriptionRecord
        {
            Id = Guid.NewGuid(),
            UserId = userId,
            SessionId = session.Id,
            Endpoint = endpoint,
            P256dh = "key",
            Auth = "auth",
            CreatedAt = now,
        };
        db.UserSessions.Add(session);
        db.PushSubscriptions.Add(subscription);
        await db.SaveChangesAsync();
        return subscription.Id;
    }

    private static async Task<HttpResponseMessage> SendMentionAsync(
        HttpClient client,
        Guid clientMessageId,
        string content) =>
        await client.PostAsJsonAsync(
            $"/api/channels/{TestData.GeneralChannelId}/messages",
            new
            {
                clientMessageId,
                content,
                attachmentIds = Array.Empty<string>(),
            });

    private static async Task<Guid> OpenConversationAsync(HttpClient client, Guid peerId)
    {
        var response = await client.PostAsJsonAsync("/api/social/conversations", new { userId = peerId });
        response.EnsureSuccessStatusCode();
        using var json = JsonDocument.Parse(await response.Content.ReadAsStringAsync());
        return Guid.Parse(json.RootElement.GetProperty("id").GetString()!);
    }

    private static async Task<HttpResponseMessage> SendDirectMessageAsync(
        HttpClient client,
        Guid conversationId,
        Guid clientMessageId,
        string content) =>
        await client.PostAsJsonAsync(
            $"/api/social/conversations/{conversationId}/messages",
            new
            {
                clientMessageId,
                content,
                attachmentIds = Array.Empty<string>(),
            });

    private static async Task<Guid> ReadIdAsync(HttpResponseMessage response)
    {
        using var json = JsonDocument.Parse(await response.Content.ReadAsStringAsync());
        return Guid.Parse(json.RootElement.GetProperty("id").GetString()!);
    }

    private static async Task<NotificationDelivery[]> DeliveryRowsAsync(
        StellarrWebApplicationFactory factory,
        Guid sourceMessageId)
    {
        await using var scope = factory.OpenScope(out var db);
        return await db.NotificationDeliveries.AsNoTracking()
            .Where(x => x.SourceMessageId == sourceMessageId)
            .OrderBy(x => x.Channel)
            .ToArrayAsync();
    }

    private static async Task<int> DispatchAsync(
        StellarrWebApplicationFactory factory,
        DateTime? utcNow = null)
    {
        await using var scope = factory.Services.CreateAsyncScope();
        var dispatcher = scope.ServiceProvider.GetRequiredService<NotificationDeliveryDispatcher>();
        return await dispatcher.DispatchAsync(utcNow ?? DateTime.UtcNow.AddMinutes(1), CancellationToken.None);
    }
}

internal sealed record SentPush(Guid SubscriptionId, NotificationPushPayload Payload);

internal sealed class FakeWebPushDelivery : IWebPushDelivery
{
    private readonly List<SentPush> _sent = [];

    internal Func<PushSubscriptionRecord, PushDeliveryResult> ResultFor { get; set; } =
        _ => PushDeliveryResult.Delivered;

    internal int Calls { get; private set; }

    internal IReadOnlyList<SentPush> Sent
    {
        get
        {
            lock (_sent)
            {
                return _sent.ToArray();
            }
        }
    }

    public Task<PushDeliveryResult> SendAsync(
        PushSubscriptionRecord subscription,
        NotificationPushPayload payload,
        CancellationToken cancellationToken)
    {
        Calls++;
        var result = ResultFor(subscription);
        if (result == PushDeliveryResult.Delivered)
        {
            lock (_sent)
            {
                _sent.Add(new SentPush(subscription.Id, payload));
            }
        }

        return Task.FromResult(result);
    }
}
