using System.Text.Json;
using Stellarr.Api.Messaging;

namespace Stellarr.Api.Tests;

/// <summary>
/// The API reads the same message corpus as the web renderer. A case added
/// there must satisfy both implementations, so the renderer and the mention
/// extractor cannot drift apart on escapes, addresses, links or marker runs.
/// </summary>
public sealed class MessageSyntaxContractTests
{
    private const string ContractPath = "test-contracts/message-syntax.json";

    public static TheoryData<string, string, string[]> SharedMessages()
    {
        var path = Path.Combine(AppContext.BaseDirectory, ContractPath);
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        var root = document.RootElement;
        if (root.ValueKind != JsonValueKind.Object ||
            !root.TryGetProperty("messages", out var messages) ||
            messages.ValueKind != JsonValueKind.Array)
        {
            throw new InvalidDataException($"{path} has no messages array.");
        }

        var data = new TheoryData<string, string, string[]>();
        foreach (var message in messages.EnumerateArray())
        {
            if (message.ValueKind != JsonValueKind.Object ||
                !message.TryGetProperty("name", out var name) ||
                name.ValueKind != JsonValueKind.String ||
                !message.TryGetProperty("content", out var content) ||
                content.ValueKind != JsonValueKind.String ||
                !message.TryGetProperty("mentions", out var mentions) ||
                mentions.ValueKind != JsonValueKind.Array)
            {
                throw new InvalidDataException($"{path} has a malformed message case.");
            }

            var expected = new List<string>();
            foreach (var mention in mentions.EnumerateArray())
            {
                if (mention.ValueKind != JsonValueKind.String)
                {
                    throw new InvalidDataException($"{path} has a non-string mention in \"{name.GetString()}\".");
                }

                expected.Add(mention.GetString()!);
            }

            data.Add(name.GetString()!, content.GetString()!, [.. expected]);
        }

        if (data.Count == 0)
        {
            throw new InvalidDataException($"{path} has no message cases.");
        }

        return data;
    }

    [Theory]
    [MemberData(nameof(SharedMessages))]
    public void SharedMessageMentions(string name, string content, string[] expected)
    {
        Assert.False(string.IsNullOrWhiteSpace(name));
        var actual = MessageMentions.ExtractNormalizedUsernames(content);
        Assert.Equal(expected.Order(StringComparer.Ordinal), actual.Order(StringComparer.Ordinal));
    }
}
