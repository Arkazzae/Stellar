using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Text.Json;

namespace Stellarr.Api.Mail;

/// <summary>Sends mail through the Resend HTTP API.</summary>
public sealed class ResendEmailDelivery(HttpClient http)
{
    /// <exception cref="InvalidOperationException">No API key is configured.</exception>
    /// <exception cref="HttpRequestException">The provider refused the message or answered with no id.</exception>
    public async Task<string> SendAsync(EmailMessage message, string apiKey, CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(apiKey))
        {
            throw new InvalidOperationException("The Resend API key is not configured.");
        }

        var fields = new Dictionary<string, object>
        {
            ["from"] = message.From,
            ["to"] = new[] { message.To },
            ["subject"] = message.Subject,
            ["html"] = message.Html,
            ["text"] = message.Text,
        };
        if (message.InlineImages.Count > 0)
        {
            // Resend embeds an attachment that has a content_id, and the HTML
            // shows it through cid: and that id.
            fields["attachments"] = message.InlineImages
                .Select(image => new
                {
                    content = Convert.ToBase64String(image.Content),
                    filename = image.FileName,
                    content_type = image.ContentType,
                    content_id = image.ContentId,
                })
                .ToArray();
        }

        using var request = new HttpRequestMessage(HttpMethod.Post, "emails")
        {
            Content = JsonContent.Create(fields),
        };
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", apiKey);
        // A retry of the same message must not be delivered twice.
        request.Headers.TryAddWithoutValidation("Idempotency-Key", message.IdempotencyKey);

        using var response = await http.SendAsync(request, cancellationToken);
        if (!response.IsSuccessStatusCode)
        {
            var detail = await response.Content.ReadAsStringAsync(cancellationToken);
            throw new HttpRequestException(
                $"Resend rejected the message ({(int)response.StatusCode}): {detail}",
                null,
                response.StatusCode);
        }

        await using var body = await response.Content.ReadAsStreamAsync(cancellationToken);
        using var payload = await JsonDocument.ParseAsync(body, cancellationToken: cancellationToken);
        if (!payload.RootElement.TryGetProperty("id", out var id) || id.ValueKind != JsonValueKind.String)
        {
            throw new HttpRequestException("Resend did not return a message id.");
        }

        return id.GetString()!;
    }
}
