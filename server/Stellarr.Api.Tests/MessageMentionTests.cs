using System.Diagnostics;
using Stellarr.Api.Messaging;

namespace Stellarr.Api.Tests;

public sealed class MessageMentionTests
{
    [Theory]
    [InlineData("Hej @Ada", new[] { "ADA" })]
    [InlineData("@ada @ADA @Ada", new[] { "ADA" })]
    [InlineData("@maja i @jo-1, @ilya_x", new[] { "MAJA", "JO-1", "ILYA_X" })]
    [InlineData("(@ada)", new[] { "ADA" })]
    [InlineData("**@ada**", new[] { "ADA" })]
    [InlineData("~~@ada~~", new[] { "ADA" })]
    [InlineData("||@ada||", new[] { "ADA" })]
    [InlineData("<b>@ada</b>", new[] { "ADA" })]
    [InlineData("<code>@ada</code>", new[] { "ADA" })]
    [InlineData("@żaneta", new[] { "ŻANETA" })]
    [InlineData("@Мая", new[] { "МАЯ" })]
    [InlineData("@ada,", new[] { "ADA" })]
    [InlineData("@ada.x", new[] { "ADA.X" })]
    [InlineData("@adahttps://example.test/x", new[] { "ADA" })]
    public void NamesInProseAreMentions(string content, string[] expected) =>
        Assert.Equal(expected.Order(), MessageMentions.ExtractNormalizedUsernames(content).Order());

    [Theory]
    [InlineData("")]
    [InlineData("@")]
    [InlineData("@a")]
    [InlineData("maja@ada.pl")]
    public void ShortNamesAndMailAddressesAreNotMentions(string content) =>
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames(content));

    [Theory]
    [InlineData("`@ada`")]
    [InlineData("``@ada``")]
    [InlineData("```@ada```")]
    [InlineData("`` `@ada` ``")]
    [InlineData("`a``@ada``")]
    [InlineData("``a`@ada``")]
    [InlineData("HTML `<code>@ada</code>`")]
    [InlineData("<code>`@ada`</code>")]
    public void CodeSpansHideNames(string content) =>
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames(content));

    [Theory]
    [InlineData("`@ada", new[] { "ADA" })]
    [InlineData("``@ada", new[] { "ADA" })]
    [InlineData("`@ada``", new[] { "ADA" })]
    [InlineData("``@ada`", new[] { "ADA" })]
    [InlineData("``a` @ada", new[] { "ADA" })]
    public void AnUnmatchedRunStaysLiteralAndDoesNotHideWhatFollows(string content, string[] expected) =>
        Assert.Equal(expected.Order(), MessageMentions.ExtractNormalizedUsernames(content).Order());

    [Theory]
    [InlineData("```\n@ada\n```")]
    [InlineData("```text\n@ada\n```")]
    [InlineData("```@ada\ncode\n```")]
    [InlineData("```\n@ada\n``` \t")]
    [InlineData("````\n@ada\n```")]
    public void FencedBlocksHideNames(string content) =>
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames(content));

    [Theory]
    [InlineData("```@ada", new[] { "ADA" })]
    [InlineData("```\n@ada", new[] { "ADA" })]
    [InlineData("```\n@ada\n```\n@bea", new[] { "BEA" })]
    [InlineData("```\n@ada\n```\n@bea\n```\n@cee\n```", new[] { "BEA" })]
    [InlineData(
        "` start\n```\nnot here\n```\n@ada\n```\nnot here\n```\nend `",
        new[] { "ADA" })]
    public void OnlyARealFencedBlockHidesAName(string content, string[] expected) =>
        Assert.Equal(expected.Order(), MessageMentions.ExtractNormalizedUsernames(content).Order());

    [Theory]
    [InlineData(@"\@ada")]
    [InlineData(@"a\@ada")]
    [InlineData(@"\@ada \@bea")]
    public void AnEscapedAtIsNotAMention(string content) =>
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames(content));

    [Theory]
    [InlineData(@"\\@ada", new[] { "ADA" })]
    [InlineData(@"\*@ada", new[] { "ADA" })]
    [InlineData(@"\_@ada", new[] { "ADA" })]
    [InlineData(@"\.@ada", new[] { "ADA" })]
    [InlineData(@"\-@ada", new[] { "ADA" })]
    [InlineData(@"\@@ada", new[] { "ADA" })]
    [InlineData(@"\@ada \@bea @cee", new[] { "CEE" })]
    public void AnEscapedCharacterDoesNotJoinTheWordAroundAMention(string content, string[] expected) =>
        Assert.Equal(expected.Order(), MessageMentions.ExtractNormalizedUsernames(content).Order());

    [Theory]
    [InlineData("https://example.test/@ada")]
    [InlineData("http://example.test/@ada")]
    [InlineData("https://example.test/a/@ada?x=1")]
    [InlineData("https://example.test/@ada)")]
    [InlineData("https://example.test/[y]@ada")]
    [InlineData("https://example.test/@ada]@bea")]
    [InlineData("xhttps://example.test/@ada")]
    [InlineData("https://example.test/@ada@bea")]
    public void ABareAddressHidesItsNames(string content) =>
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames(content));

    [Theory]
    [InlineData("HTTP://example.test/@ada", new[] { "ADA" })]
    [InlineData("https://example.test/@ada\n@bea", new[] { "BEA" })]
    [InlineData("https://example.test/@ada. @bea", new[] { "BEA" })]
    [InlineData("https://example.test/@ada, @bea", new[] { "BEA" })]
    [InlineData("https://example.test/@ada?x=@bea @cee", new[] { "CEE" })]
    public void AnAddressEndsLikeTheTranscriptSaysItDoes(string content, string[] expected) =>
        Assert.Equal(expected.Order(), MessageMentions.ExtractNormalizedUsernames(content).Order());

    [Theory]
    [InlineData("[@ada](https://example.test/x)")]
    [InlineData("[see @ada](http://example.test/x)")]
    [InlineData("[@ada](https://example.test/a_(b))")]
    [InlineData("[@ada](HTTPS://example.test/x)")]
    [InlineData("[@ada](http://example.test/@bea)")]
    [InlineData("[https://example.test/@ada](http://example.test/y)")]
    [InlineData("[[@ada](https://inner.test)](https://outer.test)")]
    [InlineData(@"\![@ada](https://example.test/x)")]
    public void ValidHttpLinksHideLabelsAndTargets(string content) =>
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames(content));

    [Theory]
    [InlineData("[@ada](https://example.test/x) @bea", new[] { "BEA" })]
    [InlineData("[x](https://outer.test) [@ada](ftp://x)", new[] { "ADA" })]
    [InlineData("[@ada](https://x) @bea [@cee](ftp://y)", new[] { "BEA", "CEE" })]
    [InlineData("[a](https://x)@ada", new[] { "ADA" })]
    public void NamesOutsideAValidLinkAreMentions(string content, string[] expected) =>
        Assert.Equal(expected.Order(), MessageMentions.ExtractNormalizedUsernames(content).Order());

    [Theory]
    [InlineData("[@ada](ftp://example.test/x)", new[] { "ADA" })]
    [InlineData("[@ada](mailto:ada@example.test)", new[] { "ADA" })]
    [InlineData("[@ada](https://example.test/x", new[] { "ADA" })]
    [InlineData("[@ada] (https://example.test/x)", new[] { "ADA" })]
    [InlineData("[@ada](www.example.test)", new[] { "ADA" })]
    [InlineData("[@ada](#anchor)", new[] { "ADA" })]
    [InlineData("[@ada](https://example.test/a b)", new[] { "ADA" })]
    [InlineData("[@ada](http://)", new[] { "ADA" })]
    [InlineData("![@ada](https://example.test/x)", new[] { "ADA" })]
    [InlineData("![@ada](ftp://example.test/x)", new[] { "ADA" })]
    [InlineData("[@ada](ftp://x/@bea)", new[] { "ADA", "BEA" })]
    [InlineData("[@ada](ftp://x) @bea", new[] { "ADA", "BEA" })]
    public void MalformedNonHttpLinksAndImagesStayLiteral(string content, string[] expected) =>
        Assert.Equal(expected.Order(), MessageMentions.ExtractNormalizedUsernames(content).Order());

    [Theory]
    [InlineData("_@ada_", new[] { "ADA" })]
    [InlineData("__@ada__", new[] { "ADA" })]
    [InlineData("_@ada_ @ada_", new[] { "ADA", "ADA_" })]
    [InlineData("__@ada__ @ada__", new[] { "ADA", "ADA__" })]
    [InlineData("_@ada__", new[] { "ADA" })]
    [InlineData("__@ada_", new[] { "ADA" })]
    [InlineData("___@ada___", new[] { "ADA" })]
    [InlineData("@ada_", new[] { "ADA_" })]
    [InlineData("@ada__", new[] { "ADA__" })]
    [InlineData("@a_da", new[] { "A_DA" })]
    [InlineData("_https://x_). @ada_", new[] { "ADA_" })]
    [InlineData("_https://x)_ @ada_", new[] { "ADA_" })]
    [InlineData("__https://x__). @ada__", new[] { "ADA__" })]
    [InlineData(@"\_@ada_", new[] { "ADA_" })]
    [InlineData("**_@ada_**", new[] { "ADA" })]
    [InlineData("~~_@ada_~~", new[] { "ADA" })]
    [InlineData("||_@ada_||", new[] { "ADA" })]
    [InlineData("_**@ada**_", new[] { "ADA" })]
    [InlineData("_~~@ada~~_", new[] { "ADA" })]
    [InlineData("_||@ada||_", new[] { "ADA" })]
    [InlineData("*@ada_*", new[] { "ADA_" })]
    [InlineData("_@ada *x_ y*", new[] { "ADA" })]
    [InlineData("_x ~~@ada_ y~~", new[] { "ADA" })]
    [InlineData("_x ||@ada_ y||", new[] { "ADA" })]
    public void PairedUnderscoresFormatTheNameInsteadOfNamingAnotherMember(string content, string[] expected) =>
        Assert.Equal(expected.Order(), MessageMentions.ExtractNormalizedUsernames(content).Order());

    [Theory]
    [InlineData("_@ada")]
    [InlineData(@"_@ada\_")]
    [InlineData("foo_@ada_")]
    [InlineData("_@ada_bar")]
    [InlineData("* _@ada* _")]
    [InlineData("*x _@ada* y_")]
    [InlineData("~~x _@ada~~ y_")]
    [InlineData("||x _@ada|| y_")]
    public void UnpairedOrCrossedUnderscoresRemainNameCharacters(string content) =>
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames(content));

    [Theory]
    [InlineData("[https://x.test](http://y)@ada", new[] { "ADA" })]
    [InlineData("[https://x.test/@bea](http://y)@ada", new[] { "ADA" })]
    [InlineData("[http://x](http://y)@ada [x](http://z)@bea", new[] { "ADA", "BEA" })]
    [InlineData("_[http://x](http://y)@ada_", new[] { "ADA" })]
    [InlineData("[a](http://x)@ada_", new[] { "ADA_" })]
    [InlineData("[a](http://x)_@ada_", new[] { "ADA" })]
    public void AUrlInsideALabelDoesNotSwallowTheMentionAfterTheLink(string content, string[] expected) =>
        Assert.Equal(expected.Order(), MessageMentions.ExtractNormalizedUsernames(content).Order());

    [Theory]
    [InlineData("[http://x](@ada)")]
    [InlineData("[https://x.test/@ada](@bea)")]
    [InlineData("https://x.test/@ada_")]
    [InlineData("_https://x.test/@ada_")]
    public void AUrlInLiteralTextStillHidesAllOfItsNames(string content) =>
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames(content));

    [Fact]
    public void ANameIsTheWholeWord()
    {
        Assert.Equal(
            [new string('X', 32)],
            MessageMentions.ExtractNormalizedUsernames($"@{new string('x', 32)}"));
        Assert.Equal(
            [new string('X', 31) + "M"],
            MessageMentions.ExtractNormalizedUsernames($"@{new string('x', 31)}m"));
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames($"@{new string('x', 33)}"));
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames($"@{new string('x', 40)}"));
        Assert.Empty(MessageMentions.ExtractNormalizedUsernames($"@{new string('x', 32)}more"));
    }

    [Fact]
    public void ALinkEndsAtTheNextWhitespace() =>
        Assert.Equal(
            ["ADA"],
            MessageMentions.ExtractNormalizedUsernames("https://example.test/@jo\n@ada"));

    [Fact]
    public void HostileTextIsReadInLinearTime()
    {
        var contents = new[]
        {
            new string('`', 100_000),
            new string('*', 100_000),
            string.Concat(Enumerable.Repeat("_@ada_ @ada_ ", 10_000)),
            string.Concat(Enumerable.Repeat("*x _@ada* y_ ", 10_000)),
            string.Concat(Enumerable.Repeat("[http://x](http://y)@ada ", 8_000)),
            new string('[', 100_000),
            string.Concat(Enumerable.Repeat("`a", 50_000)),
            string.Concat(Enumerable.Repeat("[x](http://a(", 20_000)),
            string.Concat(Enumerable.Repeat("http://a)*", 10_000)),
            string.Concat(Enumerable.Repeat("```\n@ada\n", 16_000)),
            string.Concat(Enumerable.Repeat(@"\_", 50_000)) + "@ada",
        };

        var watch = Stopwatch.StartNew();
        foreach (var content in contents)
        {
            MessageMentions.ExtractNormalizedUsernames(content);
        }

        watch.Stop();
        Assert.True(
            watch.Elapsed < TimeSpan.FromSeconds(10),
            $"The hostile bodies took {watch.Elapsed}, which looks super-linear.");
    }
}
