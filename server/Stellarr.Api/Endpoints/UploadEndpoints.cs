using System.Globalization;
using System.Security.Claims;
using Microsoft.AspNetCore.Mvc;
using Microsoft.Extensions.Options;
using Stellarr.Api.Auth;
using Stellarr.Api.Contracts;
using Stellarr.Api.Data;
using Stellarr.Api.Domain;
using Stellarr.Api.Localization;
using Stellarr.Api.Storage;

namespace Stellarr.Api.Endpoints;

/// <summary>Accepts files that members attach to messages.</summary>
public static class UploadEndpoints
{
    private const long MaximumPosterBytes = 2 * 1024 * 1024;
    private const long MaximumPreviewBytes = 1024 * 1024;
    private const int MaximumPlaceholderBytes = 32;
    private const double MaximumMediaDurationSeconds = 30 * 24 * 60 * 60;
    private const int MaximumMediaDimension = 32_768;
    private const int MaximumFileNameStemLength = 200;

    public static IEndpointRouteBuilder MapUploadEndpoints(this IEndpointRouteBuilder endpoints)
    {
        var api = endpoints.MapGroup("/api")
            .RequireAuthorization()
            .AddEndpointFilter<ServerMembershipFilter>();
        api.MapPost("/uploads", UploadAsync)
            .DisableAntiforgery()
            .RequireRateLimiting(RateLimitPolicies.Uploads);
        // Clients deployed before uploads accepted more than images still use this address.
        api.MapPost("/uploads/images", UploadAsync)
            .DisableAntiforgery()
            .RequireRateLimiting(RateLimitPolicies.Uploads);
        api.MapPost("/uploads/encrypted", UploadEncryptedAsync)
            .DisableAntiforgery()
            .RequireRateLimiting(RateLimitPolicies.Uploads);

        return endpoints;
    }

    private static async Task<IResult> UploadAsync(
        [FromForm] IFormFile file,
        [FromForm] double? durationSeconds,
        [FromForm] int? width,
        [FromForm] int? height,
        [FromForm] IFormFile? poster,
        [FromForm] IFormFile? preview,
        [FromForm] string? placeholder,
        [FromForm] bool? voiceMessage,
        ClaimsPrincipal principal,
        AppDbContext db,
        MediaStorage mediaStorage,
        UploadAdmissionGate admissionGate,
        IOptions<UploadOptions> uploadOptions,
        Messages messages,
        CancellationToken cancellationToken)
    {
        var inspection = await mediaStorage.InspectAttachmentAsync(file, uploadOptions.Value, cancellationToken);
        if (!inspection.Accepted)
        {
            return Refusal(inspection, messages);
        }

        var spec = inspection.Spec;
        var isVoiceMessage = voiceMessage == true;
        if (MetadataProblem(spec, isVoiceMessage, durationSeconds, width, height, messages) is { } problem)
        {
            return problem;
        }

        var id = Guid.NewGuid();
        var month = DateTimeOffset.UtcNow.ToString("yyyy/MM", CultureInfo.InvariantCulture);
        PosterUpload? posterUpload = null;
        if (poster is not null)
        {
            var posterInspection = await mediaStorage.InspectImageAsync(poster, MaximumPosterBytes, cancellationToken);
            if (spec.Kind != UploadKind.Video || !posterInspection.Accepted)
            {
                return Invalid("poster", messages.Get(MessageKey.UploadPosterInvalid));
            }

            posterUpload = new PosterUpload(
                poster,
                posterInspection.Spec,
                $"posters/{month}/{id:N}{posterInspection.Spec.Extension}");
        }

        PreviewUpload? previewUpload = null;
        if (preview is not null)
        {
            var previewInspection = await mediaStorage.InspectImageAsync(preview, MaximumPreviewBytes, cancellationToken);
            if (spec.Kind != UploadKind.Image ||
                !previewInspection.Accepted ||
                previewInspection.Spec.ContentType is not ("image/webp" or "image/jpeg"))
            {
                return Invalid("preview", messages.Get(MessageKey.UploadPreviewInvalid));
            }

            previewUpload = new PreviewUpload(
                preview,
                previewInspection.Spec,
                $"previews/{month}/{id:N}{previewInspection.Spec.Extension}");
        }

        if (placeholder is not null && (spec.Kind != UploadKind.Image || !IsPlaceholder(placeholder)))
        {
            return Invalid("placeholder", messages.Get(MessageKey.UploadPlaceholderInvalid));
        }

        var userId = principal.RequireUserId();
        var admission = await admissionGate.TryEnterAsync(
            db,
            userId,
            file.Length + (posterUpload?.File.Length ?? 0) + (previewUpload?.File.Length ?? 0),
            cancellationToken);
        if (!admission.Accepted)
        {
            return NoRoom(admission.Failure, messages);
        }

        await using var lease = admission.Lease;

        var key = $"{spec.Folder}/{month}/{id:N}{spec.Extension}";
        var attachment = new ImageAttachment
        {
            Id = id,
            UploadedById = userId,
            ObjectKey = key,
            FileName = SafeFileName(file.FileName, spec),
            ContentType = spec.ContentType,
            SizeBytes = file.Length,
            Width = width,
            Height = height,
            DurationSeconds = durationSeconds is { } measured ? Math.Round(measured, 3) : null,
            IsVoiceMessage = isVoiceMessage,
            PosterObjectKey = posterUpload?.Key,
            PosterContentType = posterUpload?.Spec.ContentType,
            PosterSizeBytes = posterUpload?.File.Length,
            PreviewObjectKey = previewUpload?.Key,
            PreviewContentType = previewUpload?.Spec.ContentType,
            PreviewSizeBytes = previewUpload?.File.Length,
            Placeholder = placeholder,
            CreatedAt = DateTime.UtcNow,
        };

        try
        {
            await mediaStorage.StoreAsync(key, file, cancellationToken);
            if (posterUpload is not null)
            {
                await mediaStorage.StoreAsync(posterUpload.Key, posterUpload.File, cancellationToken);
            }

            if (previewUpload is not null)
            {
                await mediaStorage.StoreAsync(previewUpload.Key, previewUpload.File, cancellationToken);
            }

            db.ImageAttachments.Add(attachment);
            await db.SaveChangesAsync(cancellationToken);
        }
        catch
        {
            await mediaStorage.RollbackAsync([key, posterUpload?.Key, previewUpload?.Key]);
            throw;
        }

        return Results.Created($"/assets/{id}", attachment.ToDto());
    }

    /// <summary>
    /// Accepts a file a browser encrypted for an encrypted conversation, or the
    /// poster of such a video. The bytes are opaque: only their size is checked,
    /// against the largest size any kind of upload may have plus the
    /// authentication tag, and against the storage quotas. Nothing is read from
    /// them, and nothing about them is stored but their size.
    /// </summary>
    private static async Task<IResult> UploadEncryptedAsync(
        [FromForm] IFormFile file,
        ClaimsPrincipal principal,
        AppDbContext db,
        MediaStorage mediaStorage,
        UploadAdmissionGate admissionGate,
        IOptions<UploadOptions> uploadOptions,
        Messages messages,
        CancellationToken cancellationToken)
    {
        if (file.Length <= 0)
        {
            return Invalid("file", messages.Get(MessageKey.FileEmpty));
        }

        if (file.Length > MaxEncryptedBytes(uploadOptions.Value))
        {
            return Results.Problem(
                statusCode: StatusCodes.Status413PayloadTooLarge,
                title: messages.Get(MessageKey.UploadTooLarge),
                detail: messages.Get(MessageKey.UploadOverLimit));
        }

        var userId = principal.RequireUserId();
        var admission = await admissionGate.TryEnterAsync(db, userId, file.Length, cancellationToken);
        if (!admission.Accepted)
        {
            return NoRoom(admission.Failure, messages);
        }

        await using var lease = admission.Lease;

        var id = Guid.NewGuid();
        var key = $"encrypted/{DateTimeOffset.UtcNow.ToString("yyyy/MM", CultureInfo.InvariantCulture)}/{id:N}.bin";
        var attachment = new ImageAttachment
        {
            Id = id,
            UploadedById = userId,
            ObjectKey = key,
            FileName = "encrypted.bin",
            ContentType = EncryptedContentType,
            SizeBytes = file.Length,
            IsEncrypted = true,
            CreatedAt = DateTime.UtcNow,
        };

        try
        {
            await mediaStorage.StoreAsync(key, file, cancellationToken);
            db.ImageAttachments.Add(attachment);
            await db.SaveChangesAsync(cancellationToken);
        }
        catch
        {
            await mediaStorage.RollbackAsync([key]);
            throw;
        }

        return Results.Created($"/assets/{id}", attachment.ToDto());
    }

    /// <summary>What every encrypted upload is stored and served as.</summary>
    internal const string EncryptedContentType = "application/octet-stream";

    /// <summary>The largest plain upload, encrypted: AES-GCM adds its tag and nothing else.</summary>
    internal static long MaxEncryptedBytes(UploadOptions options) =>
        new[] { options.MaxImageBytes, options.MaxAudioBytes, options.MaxVideoBytes, options.MaxFileBytes }.Max() +
        EncryptionLimits.TagBytes;

    /// <summary>The still image that stands in for a video before it plays.</summary>
    private sealed record PosterUpload(IFormFile File, UploadSpec Spec, string Key);

    /// <summary>The reduced copy of an image that a small tile shows instead of the original.</summary>
    private sealed record PreviewUpload(IFormFile File, UploadSpec Spec, string Key);

    /// <summary>
    /// Whether the text is standard padded base64 of one to
    /// <see cref="MaximumPlaceholderBytes"/> bytes. Only the shape is checked;
    /// the bytes mean something to clients alone.
    /// </summary>
    private static bool IsPlaceholder(string value)
    {
        if (value.Length == 0 || value.Length % 4 != 0 || value.Any(char.IsWhiteSpace))
        {
            return false;
        }

        Span<byte> decoded = stackalloc byte[MaximumPlaceholderBytes];
        // A text that decodes to more than fits is refused by the same call.
        return Convert.TryFromBase64String(value, decoded, out var written) && written > 0;
    }

    private static IResult Refusal(UploadInspection inspection, Messages messages) => inspection.Failure switch
    {
        UploadFailure.Empty => Invalid("file", messages.Get(MessageKey.FileEmpty)),
        UploadFailure.UnsupportedType => Results.Problem(
            statusCode: StatusCodes.Status415UnsupportedMediaType,
            title: messages.Get(MessageKey.UploadTypeUnsupported),
            detail: messages.Get(MessageKey.UploadTypesAllowed)),
        UploadFailure.TooLarge => Results.Problem(
            statusCode: StatusCodes.Status413PayloadTooLarge,
            title: messages.Get(MessageKey.UploadTooLarge),
            detail: messages.Get(inspection.Spec?.Kind switch
            {
                UploadKind.Image => MessageKey.UploadImageMaximum,
                UploadKind.Audio => MessageKey.UploadAudioMaximum,
                UploadKind.Video => MessageKey.UploadVideoMaximum,
                UploadKind.File => MessageKey.UploadFileMaximum,
                _ => MessageKey.UploadOverLimit,
            })),
        // A signature that does not fit the declared type.
        _ => Results.Problem(
            statusCode: StatusCodes.Status415UnsupportedMediaType,
            title: messages.Get(MessageKey.UploadTypeMismatch),
            detail: messages.Get(MessageKey.FileContentMismatch)),
    };

    /// <summary>
    /// Checks the properties the client measured: duration, dimensions and
    /// whether the file is a recorded voice message. Null when all of them fit the file.
    /// </summary>
    private static IResult? MetadataProblem(
        UploadSpec spec,
        bool isVoiceMessage,
        double? durationSeconds,
        int? width,
        int? height,
        Messages messages)
    {
        if (isVoiceMessage &&
            (spec.Kind != UploadKind.Audio ||
             durationSeconds is not { } voiceDuration ||
             !double.IsFinite(voiceDuration) ||
             voiceDuration <= 0 ||
             voiceDuration > MessageLimits.MaxVoiceMessageDurationSeconds))
        {
            return Invalid(
                "voiceMessage",
                messages.Get(
                    MessageKey.VoiceMessageInvalid,
                    ("minutes", (MessageLimits.MaxVoiceMessageDurationSeconds / 60).ToString("0", CultureInfo.CurrentCulture))));
        }

        if (durationSeconds is { } duration &&
            (spec.Kind is UploadKind.Image or UploadKind.File ||
             !double.IsFinite(duration) ||
             duration <= 0 ||
             duration > MaximumMediaDurationSeconds))
        {
            return Invalid("durationSeconds", messages.Get(MessageKey.MediaDurationInvalid));
        }

        if (width.HasValue != height.HasValue ||
            width is <= 0 or > MaximumMediaDimension ||
            height is <= 0 or > MaximumMediaDimension ||
            (width.HasValue && spec.Kind is UploadKind.Audio or UploadKind.File))
        {
            return Invalid("dimensions", messages.Get(MessageKey.MediaDimensionsInvalid));
        }

        return null;
    }

    internal static IResult NoRoom(UploadAdmissionFailure failure, Messages messages) => Results.Problem(
        statusCode: StatusCodes.Status507InsufficientStorage,
        title: messages.Get(MessageKey.StorageFull),
        detail: messages.Get(failure switch
        {
            UploadAdmissionFailure.UserQuota => MessageKey.StorageUserQuota,
            UploadAdmissionFailure.InstanceQuota => MessageKey.StorageInstanceQuota,
            UploadAdmissionFailure.LowDiskSpace => MessageKey.StorageLowDisk,
            _ => MessageKey.StorageUnavailable,
        }));

    private static IResult Invalid(string field, string message) =>
        Results.ValidationProblem(new Dictionary<string, string[]> { [field] = [message] });

    /// <summary>
    /// The name a recipient downloads the file as. The extension comes from the
    /// verified type, never from the submitted name, so that a PDF cannot be
    /// handed out as <c>invoice.exe</c>.
    /// </summary>
    private static string SafeFileName(string submittedName, UploadSpec spec)
    {
        var stem = Path.GetFileNameWithoutExtension(Path.GetFileName(submittedName));
        var safeStem = new string(stem
            .Where(value => char.IsLetterOrDigit(value) || value is ' ' or '-' or '_' or '.')
            .Take(MaximumFileNameStemLength)
            .ToArray())
            .Trim(' ', '.');
        return $"{(safeStem.Length == 0 ? "attachment" : safeStem)}{spec.Extension}";
    }
}
