using Stellarr.Api.Integrations.Discord;

namespace Stellarr.Api.Tests;

public sealed class DiscordTemplateLinkTests
{
    [Theory]
    [InlineData("hgM48av5Q69A", "hgM48av5Q69A")]
    [InlineData("  hgM48av5Q69A\n", "hgM48av5Q69A")]
    [InlineData("https://discord.new/hgM48av5Q69A", "hgM48av5Q69A")]
    [InlineData("http://discord.new/hgM48av5Q69A", "hgM48av5Q69A")]
    [InlineData("discord.new/hgM48av5Q69A", "hgM48av5Q69A")]
    [InlineData("www.discord.new/hgM48av5Q69A", "hgM48av5Q69A")]
    [InlineData("https://discord.new/hgM48av5Q69A/", "hgM48av5Q69A")]
    [InlineData("HTTPS://Discord.New/hgM48av5Q69A", "hgM48av5Q69A")]
    [InlineData("https://discord.com/template/hgM48av5Q69A", "hgM48av5Q69A")]
    [InlineData("https://www.discord.com/template/hgM48av5Q69A", "hgM48av5Q69A")]
    [InlineData("discord.com/template/hgM48av5Q69A", "hgM48av5Q69A")]
    [InlineData("https://discordapp.com/template/hgM48av5Q69A", "hgM48av5Q69A")]
    [InlineData("www.discordapp.com/template/hgM48av5Q69A", "hgM48av5Q69A")]
    public void ReadsTheCodeOutOfEveryShapeDiscordHandsOut(string input, string expected)
    {
        Assert.True(DiscordTemplateLink.TryParse(input, out var code, out _));
        Assert.Equal(expected, code.Value);
    }

    [Theory]
    [InlineData(null, TemplateLinkRefusal.Empty)]
    [InlineData("", TemplateLinkRefusal.Empty)]
    [InlineData("   ", TemplateLinkRefusal.Empty)]
    [InlineData("https://discord.gg/abcdef", TemplateLinkRefusal.InviteLink)]
    [InlineData("discord.gg/abcdef", TemplateLinkRefusal.InviteLink)]
    [InlineData("https://discord.com/invite/abcdef", TemplateLinkRefusal.InviteLink)]
    [InlineData("https://discord.new.evil.test/hgM48av5Q69A", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://evildiscord.new/hgM48av5Q69A", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.com.evil.test/template/hgM48av5Q69A", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.new@evil.test/hgM48av5Q69A", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://evil.test@discord.new/hgM48av5Q69A", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.new:8443/hgM48av5Q69A", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.new/hgM48av5Q69A?ref=x", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.new/hgM48av5Q69A#top", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.new/hgM48av5Q69A/extra", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.com/template/hgM48av5Q69A/extra", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.com/hgM48av5Q69A", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.com/channels/1/2", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.new/", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.new", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("discord.new", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("ftp://discord.new/hgM48av5Q69A", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("javascript://discord.new/hgM48av5Q69A", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.new\\@evil.test/x", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("hgM48 av5Q69A", TemplateLinkRefusal.NotATemplateLink)]
    [InlineData("https://discord.new/..", TemplateLinkRefusal.InvalidCode)]
    [InlineData("https://discord.new/%2e%2e", TemplateLinkRefusal.InvalidCode)]
    [InlineData("https://discord.new/abc%2Fdef", TemplateLinkRefusal.InvalidCode)]
    [InlineData("x", TemplateLinkRefusal.InvalidCode)]
    [InlineData("abc!def", TemplateLinkRefusal.InvalidCode)]
    public void RefusesAnythingElseAndSaysWhy(string? input, TemplateLinkRefusal expected)
    {
        Assert.False(DiscordTemplateLink.TryParse(input, out var code, out var refusal));
        Assert.Null(code);
        Assert.Equal(expected, refusal);
    }
}
