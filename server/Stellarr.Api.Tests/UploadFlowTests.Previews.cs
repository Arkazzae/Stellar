using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.DependencyInjection.Extensions;
using Stellarr.Api.Domain;
using Stellarr.Api.Storage;

namespace Stellarr.Api.Tests;

/// <summary>The reduced copy and the tiny placeholder an image upload may carry.</summary>
public sealed partial class UploadFlowTests
{
    private const string Placeholder = "AQIDBAUGBwg=";
    private const string MessageChannel = "20000000-0000-0000-0000-000000000001";

    private static readonly byte[] Webp = "RIFF\0\0\0\0WEBPVP8 "u8.ToArray();

    [Theory]
    [InlineData("image/webp", ".webp")]
    [InlineData("image/jpeg", ".jpg")]
    public async Task AnImageMayCarryAPreviewAndAPlaceholderThatComeBackInItsMessage(string type, string extension)
    {
        using var maja = await SignedInAsync(factory, "maja", "Aurora!2026");
        var bytes = type == "image/webp" ? Webp : Jpeg;
        using var form = Form(Png, "image/png", "photo.png", ("width", "4000"), ("height", "3000"), ("placeholder", Placeholder));
        form.Add(Image(bytes, type), "preview", $"preview{extension}");

        var response = await maja.PostAsync("/api/uploads", form);

        Assert.Equal(HttpStatusCode.Created, response.StatusCode);
        var attachment = await ReadAsync(response);
        var id = attachment.GetProperty("id").GetString()!;
        Assert.Equal($"/assets/{id}/preview", attachment.GetProperty("previewUrl").GetString());
        Assert.Equal(Placeholder, attachment.GetProperty("placeholder").GetString());
        Assert.Equal(4000, attachment.GetProperty("width").GetInt32());
        Assert.Equal(JsonValueKind.Null, attachment.GetProperty("posterUrl").ValueKind);

        await using (var scope = factory.OpenScope(out var db))
        {
            var row = await db.ImageAttachments.SingleAsync(x => x.Id == Guid.Parse(id));
            Assert.StartsWith("previews/", row.PreviewObjectKey, StringComparison.Ordinal);
            Assert.EndsWith(extension, row.PreviewObjectKey, StringComparison.Ordinal);
            Assert.Equal(type, row.PreviewContentType);
            Assert.Equal(bytes.Length, row.PreviewSizeBytes);
            Assert.True(Storage(scope).Contains(row.PreviewObjectKey!));
        }

        var sent = await maja.PostAsJsonAsync(
            $"/api/channels/{MessageChannel}/messages",
            new { content = "podglad", attachmentIds = new[] { id } });
        Assert.Equal(HttpStatusCode.Created, sent.StatusCode);
        var carried = (await ReadAsync(sent)).GetProperty("attachments")[0];
        Assert.Equal($"/assets/{id}/preview", carried.GetProperty("previewUrl").GetString());
        Assert.Equal(Placeholder, carried.GetProperty("placeholder").GetString());
    }

    [Fact]
    public async Task AnImageWithoutThemHasNeitherAPreviewAddressNorAPlaceholder()
    {
        using var maja = await SignedInAsync(factory, "maja", "Aurora!2026");

        var attachment = await ReadAsync(await maja.PostAsync("/api/uploads", Form(Png, "image/png", "tiny.png")));

        Assert.Equal(JsonValueKind.Null, attachment.GetProperty("previewUrl").ValueKind);
        Assert.Equal(JsonValueKind.Null, attachment.GetProperty("placeholder").ValueKind);
        var url = attachment.GetProperty("url").GetString();
        Assert.Equal(HttpStatusCode.NotFound, (await maja.GetAsync($"{url}/preview")).StatusCode);
    }

    [Fact]
    public async Task APreviewIsServedWithTheHeadersOfAPosterAndRefusedToSomeoneWithoutAccess()
    {
        using var maja = await SignedInAsync(factory, "maja", "Aurora!2026");
        using var ada = await SignedInAsync(factory, "ada", "Quasar!2026");
        using var anonymous = Client(factory);
        using var form = Form(Png, "image/png", "photo.png");
        form.Add(Image(Webp, "image/webp"), "preview", "preview.webp");
        var upload = await ReadAsync(await maja.PostAsync("/api/uploads", form));
        var previewUrl = upload.GetProperty("previewUrl").GetString()!;

        var response = await maja.GetAsync(previewUrl);

        Assert.Equal(HttpStatusCode.OK, response.StatusCode);
        Assert.Equal("image/webp", response.Content.Headers.ContentType?.MediaType);
        Assert.Equal(Webp.Length, response.Content.Headers.ContentLength);
        Assert.Equal(Webp, await response.Content.ReadAsByteArrayAsync());
        Assert.Equal(HttpStatusCode.NotFound, (await ada.GetAsync(previewUrl)).StatusCode);
        Assert.Equal(HttpStatusCode.Unauthorized, (await anonymous.GetAsync(previewUrl)).StatusCode);
    }

    [Fact]
    public async Task APreviewIsOpenedToTheMembersOfTheServerOfTheMessageThatCarriesItsImage()
    {
        using var maja = await SignedInAsync(factory, "maja", "Aurora!2026");
        using var ada = await SignedInAsync(factory, "ada", "Quasar!2026");
        using var form = Form(Png, "image/png", "photo.png");
        form.Add(Image(Jpeg, "image/jpeg"), "preview", "preview.jpg");
        var upload = await ReadAsync(await maja.PostAsync("/api/uploads", form));
        var id = upload.GetProperty("id").GetString()!;
        var previewUrl = upload.GetProperty("previewUrl").GetString()!;
        Assert.Equal(HttpStatusCode.NotFound, (await ada.GetAsync(previewUrl)).StatusCode);
        await using (var scope = factory.OpenScope(out var db))
        {
            db.MessageAttachments.Add(new MessageAttachment { MessageId = SeededMessageId, AttachmentId = Guid.Parse(id) });
            await db.SaveChangesAsync();
        }

        Assert.Equal(Jpeg, await ada.GetByteArrayAsync(previewUrl));
    }

    [Theory]
    [InlineData("video")]
    [InlineData("audio")]
    [InlineData("file")]
    public async Task APreviewIsRefusedForEveryKindButImage(string kind)
    {
        using var client = await SignedInAsync(factory, "maja", "Aurora!2026");
        using var form = kind switch
        {
            "video" => Form(Mp4, "video/mp4", "clip.mp4"),
            "audio" => Form(Wave, "audio/wav", "sound.wav"),
            _ => Form(Pdf, "application/pdf", "note.pdf"),
        };
        form.Add(Image(Jpeg, "image/jpeg"), "preview", "preview.jpg");

        var response = await client.PostAsync("/api/uploads", form);

        Assert.Equal(HttpStatusCode.BadRequest, response.StatusCode);
        Assert.True((await ReadAsync(response)).GetProperty("errors").TryGetProperty("preview", out _));
    }

    [Theory]
    [InlineData("audio")]
    [InlineData("mismatch")]
    [InlineData("empty")]
    [InlineData("tooLarge")]
    [InlineData("png")]
    public async Task APreviewThatIsNotAGoodSmallWebpOrJpegIsRefusedAndNothingIsStored(string defect)
    {
        using var client = await SignedInAsync(factory, "maja", "Aurora!2026");
        await using var scope = factory.OpenScope(out var db);
        var before = Storage(scope).Count;
        var rows = await db.ImageAttachments.CountAsync();
        using var form = Form(Png, "image/png", "photo.png");
        form.Add(
            defect switch
            {
                "audio" => Image(Wave, "audio/wav"),
                "mismatch" => Image(Png, "image/jpeg"),
                "empty" => Image([], "image/jpeg"),
                "png" => Image(Png, "image/png"),
                _ => Image([.. Jpeg, .. new byte[1024 * 1024]], "image/jpeg"),
            },
            "preview",
            "preview.jpg");

        var response = await client.PostAsync("/api/uploads", form);

        Assert.Equal(HttpStatusCode.BadRequest, response.StatusCode);
        Assert.True((await ReadAsync(response)).GetProperty("errors").TryGetProperty("preview", out _));
        Assert.Equal(before, Storage(scope).Count);
        Assert.Equal(rows, await db.ImageAttachments.CountAsync());
    }

    [Fact]
    public async Task APreviewOfExactlyOneMebibyteIsAccepted()
    {
        using var client = await SignedInAsync(factory, "maja", "Aurora!2026");
        using var form = Form(Png, "image/png", "photo.png");
        form.Add(Image([.. Jpeg, .. new byte[1024 * 1024 - Jpeg.Length]], "image/jpeg"), "preview", "preview.jpg");

        Assert.Equal(HttpStatusCode.Created, (await client.PostAsync("/api/uploads", form)).StatusCode);
    }

    [Theory]
    [InlineData("not base64!")]
    [InlineData("AQIDBA")]
    [InlineData("AQID BA==")]
    [InlineData("AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyAh")]
    [InlineData("")]
    public async Task APlaceholderThatIsNotBase64OfOneToThirtyTwoBytesIsRefused(string placeholder)
    {
        using var client = await SignedInAsync(factory, "maja", "Aurora!2026");
        using var form = Form(Png, "image/png", "photo.png", ("placeholder", placeholder));

        var response = await client.PostAsync("/api/uploads", form);

        Assert.Equal(HttpStatusCode.BadRequest, response.StatusCode);
        Assert.True((await ReadAsync(response)).GetProperty("errors").TryGetProperty("placeholder", out _));
    }

    [Fact]
    public async Task AThirtyTwoBytePlaceholderIsStoredAsUploaded()
    {
        using var client = await SignedInAsync(factory, "maja", "Aurora!2026");
        var thirtyTwo = Convert.ToBase64String(new byte[32]);
        using var form = Form(Png, "image/png", "photo.png", ("placeholder", thirtyTwo));

        var attachment = await ReadAsync(await client.PostAsync("/api/uploads", form));

        Assert.Equal(44, thirtyTwo.Length);
        Assert.Equal(thirtyTwo, attachment.GetProperty("placeholder").GetString());
    }

    [Theory]
    [InlineData("video")]
    [InlineData("audio")]
    [InlineData("file")]
    public async Task APlaceholderIsRefusedForEveryKindButImage(string kind)
    {
        using var client = await SignedInAsync(factory, "maja", "Aurora!2026");
        using var form = kind switch
        {
            "video" => Form(Mp4, "video/mp4", "clip.mp4", ("placeholder", Placeholder)),
            "audio" => Form(Wave, "audio/wav", "sound.wav", ("placeholder", Placeholder)),
            _ => Form(Pdf, "application/pdf", "note.pdf", ("placeholder", Placeholder)),
        };

        var response = await client.PostAsync("/api/uploads", form);

        Assert.Equal(HttpStatusCode.BadRequest, response.StatusCode);
        Assert.True((await ReadAsync(response)).GetProperty("errors").TryGetProperty("placeholder", out _));
    }

    [Fact]
    public async Task AFailedPreviewWriteTakesTheImageOutOfStorageAgain()
    {
        var inner = new FakeObjectStorage();
        using var faulty = new StellarrWebApplicationFactory(
            loadTestData: true,
            configureServices: services =>
            {
                services.RemoveAll<IObjectStorage>();
                services.AddSingleton<IObjectStorage>(new PosterRejectingStorage(inner, "previews/"));
            });
        using var client = await SignedInAsync(faulty, "maja", "Aurora!2026");
        using var form = Form(Png, "image/png", "photo.png");
        form.Add(Image(Jpeg, "image/jpeg"), "preview", "preview.jpg");

        var response = await client.PostAsync("/api/uploads", form);

        Assert.Equal(HttpStatusCode.InternalServerError, response.StatusCode);
        Assert.Equal(0, inner.Count);
        await using var scope = faulty.OpenScope(out var db);
        Assert.False(await db.ImageAttachments.AnyAsync());
        Assert.False(await db.ObjectDeletions.AnyAsync());
    }
}
