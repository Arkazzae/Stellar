using Stellarr.Api.Integrations.Discord;
using Stellarr.Api.Messaging;

namespace Stellarr.Api.Tests;

/// <summary>
/// Discord message text rewritten for Stellarr. Every case is also read back
/// by the mention extractor that sends notifications, which must find exactly
/// the accounts the resolver chose to mention and nobody else.
/// </summary>
public sealed class DiscordContentTests
{
    private static readonly Resolver Known = new();

    [Theory]
    // Users: the resolver decides between a mention and a label.
    [InlineData("hi <@1>", "hi @ada", "ADA")]
    [InlineData("hi <@!1>", "hi @ada", "ADA")]
    [InlineData("<@2> ok", "@ada_ ok", "ADA_")]
    [InlineData("**<@1>**", "**@ada**", "ADA")]
    [InlineData("<@3>", "Bea (left)", "")]
    [InlineData("<@4>", @"\@bad name!", "")]
    [InlineData("<@5>", @"\*\*Bob\*\*", "")]
    [InlineData("<@99>", @"\@unknown-user", "")]
    // A mention keeps its edge: Stellarr reads the whole word after the at sign.
    [InlineData("thanks <@1>.", @"thanks @ada\.", "ADA")]
    [InlineData("<@1>_", @"@ada\_", "ADA")]
    [InlineData("<@1>-ish", @"@ada\-ish", "ADA")]
    [InlineData("<@1>s", @"\@adas", "")]
    [InlineData("x<@1>", @"x\@ada", "")]
    [InlineData("<@1><@1>", @"@ada\@ada", "ADA")]
    [InlineData("<@1><t:0:d>", @"\@ada1970-01-01", "")]
    // An underscore that pairs with one outside would cut ada_ down to ada.
    [InlineData("_hi <@2>_", @"_hi \@ada__", "")]
    // Roles and channels have no mention in Stellarr; unknown ids stay readable.
    [InlineData("<@&10> please", @"\@Moderators please", "")]
    [InlineData("<@&99>", @"\@deleted-role", "")]
    [InlineData("see <#20>", "see #general", "")]
    [InlineData("<#21>", @"#http\://evil.test", "")]
    [InlineData("<#99>", "#deleted-channel", "")]
    // Custom emoji become the server's shortcode, or their own name as text.
    [InlineData("<:party:30>", ":party:", "")]
    [InlineData("<a:party:30>!", ":party:!", "")]
    [InlineData("<:party:30>s", ":party:s", "")]
    [InlineData("<:pepe_hands:99>", @":pepe\_hands:", "")]
    [InlineData("<:Party:31>", @"\:Party:", "")]
    [InlineData("x:ab<:party:30>", @"x\:ab:party:", "")]
    // Times, in UTC, as the style showed them.
    [InlineData("<t:1618953630>", "2021-04-20 21:20 UTC", "")]
    [InlineData("<t:1618953630:f>", "2021-04-20 21:20 UTC", "")]
    [InlineData("<t:1618953630:R>", "2021-04-20 21:20 UTC", "")]
    [InlineData("<t:1618953630:d>", "2021-04-20", "")]
    [InlineData("<t:1618953630:D>", "2021-04-20", "")]
    [InlineData("<t:1618953630:t>", "21:20 UTC", "")]
    [InlineData("<t:1618953630:T>", @"21\:20:30 UTC", "")]
    [InlineData("<t:1618953630:S>", @"2021-04-20 21\:20:30 UTC", "")]
    [InlineData("<t:-1>", "1969-12-31 23:59 UTC", "")]
    [InlineData("<t:99999999999999999>", "<t:99999999999999999>", "")]
    // Nothing inside code is touched.
    [InlineData("`<@1>`", "`<@1>`", "")]
    [InlineData("``a ` <@&10>``", "``a ` <@&10>``", "")]
    [InlineData("```<@1> @ada```", "```<@1> @ada```", "")]
    [InlineData("```js\nhi <@1> <:party:30> @ada\n```", "```js\nhi <@1> <:party:30> @ada\n```", "")]
    [InlineData("a `<@1>` b <@1>", "a `<@1>` b @ada", "ADA")]
    [InlineData("`unclosed <@1>", "`unclosed @ada", "ADA")]
    // Text that only Stellarr would read as a mention or an emoji stays text.
    [InlineData("@everyone and @here", @"\@everyone and \@here", "")]
    [InlineData("hey @ada", @"hey \@ada", "")]
    [InlineData("(@ada)", @"(\@ada)", "")]
    [InlineData("_@ada_", @"_\@ada_", "")]
    [InlineData("mail me@ada.pl", "mail me@ada.pl", "")]
    [InlineData("at 18:30:00", @"at 18\:30:00", "")]
    [InlineData(":party: time", @"\:party: time", "")]
    [InlineData("https://example.test/@ada:x: and @ada", @"https://example.test/@ada:x: and \@ada", "")]
    // Discord's escapes are Stellarr's escapes.
    [InlineData(@"\*not bold\* \@ada", @"\*not bold\* \@ada", "")]
    [InlineData(@"\<@1>", @"\<\@1>", "")]
    [InlineData("<@123", @"<\@123", "")]
    [InlineData("", "", "")]
    public void RewritesDiscordTokensIntoStellarrText(string discord, string expected, string mentions)
    {
        var rewritten = DiscordContent.Rewrite(discord, Known);

        Assert.Equal(expected, rewritten);
        Assert.Equal(
            mentions.Split(',', StringSplitOptions.RemoveEmptyEntries).Order(),
            MessageMentions.ExtractNormalizedUsernames(rewritten).Order());
    }

    [Fact]
    public void NoMixOfPiecesMentionsAnyoneTheResolverDidNotName()
    {
        string[] fragments =
        [
            "<@1>", "<@2>", "<@99>", "<@&10>", "<#20>", "<:party:30>", "<t:0>", "@ada", "@everyone",
            "_", "__", "*", ".", "-", "a", "1", " ", "`", "\\", "\n", "```\n", "https://x.test/", ":",
        ];

        foreach (var first in fragments)
        foreach (var second in fragments)
        foreach (var third in fragments)
        {
            var discord = first + second + third;
            var found = MessageMentions.ExtractNormalizedUsernames(DiscordContent.Rewrite(discord, Known));
            Assert.True(found.IsSubsetOf(["ADA", "ADA_"]), $"{discord} mentions {string.Join(", ", found)}");
        }
    }

    private sealed class Resolver : IDiscordReferenceResolver
    {
        public MentionText? User(string userId) => userId switch
        {
            "1" => MentionText.Account("ada"),
            "2" => MentionText.Account("ada_"),
            "3" => MentionText.Label("Bea (left)"),
            "4" => MentionText.Account("bad name!"),
            "5" => MentionText.Label("**Bob**"),
            _ => null,
        };

        public string? Role(string roleId) => roleId == "10" ? "@Moderators" : null;

        public string? Channel(string channelId) => channelId switch
        {
            "20" => "#general",
            "21" => "#http://evil.test",
            _ => null,
        };

        public string? Emoji(string emojiId) => emojiId switch
        {
            "30" => "party",
            "31" => "Party",
            _ => null,
        };
    }
}
