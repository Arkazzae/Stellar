using Stellarr.Api.Integrations.Discord;

namespace Stellarr.Api.Tests;

public sealed class DiscordCdnTests
{
    [Theory]
    [InlineData("https://cdn.discordapp.com/attachments/1012345678900020080/1234567891233211234/my_image.png?ex=65d903de&is=65c68ede&hm=2481f30dd67f503f54d020ae3b5533b9987fae4e55f2b4e3926e08a3fa3ee24f&")]
    [InlineData("https://media.discordapp.net/attachments/1/2/clip.mp4")]
    [InlineData("https://media.discordapp.net/stickers/749054660769218631.gif")]
    [InlineData("https://CDN.DiscordApp.com/emojis/41771983429993937.webp")]
    [InlineData("https://cdn.discordapp.com:443/icons/1/abc.png")]
    public void AcceptsHttpsOnTheTwoCdnHosts(string url) => Assert.True(DiscordCdn.IsCdnUrl(url));

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("/attachments/1/2/a.png")]
    [InlineData("cdn.discordapp.com/attachments/1/2/a.png")]
    [InlineData("http://cdn.discordapp.com/attachments/1/2/a.png")]
    [InlineData("ftp://cdn.discordapp.com/a.png")]
    [InlineData("https://cdn.discordapp.com.evil.test/a.png")]
    [InlineData("https://cdn.discordapp.com@evil.test/a.png")]
    [InlineData("https://evil.test@cdn.discordapp.com/a.png")]
    [InlineData("https://cdn.discordapp.com:8443/a.png")]
    [InlineData("https://evilcdn.discordapp.com/a.png")]
    [InlineData("https://images-ext-1.discordapp.net/external/x/https/evil.test/a.png")]
    [InlineData("https://discord.com/api/v10/users/@me")]
    [InlineData("https://cdn.discordapp.com./a.png")]
    [InlineData("https://127.0.0.1/a.png")]
    public void RefusesEverythingElse(string? url) => Assert.False(DiscordCdn.IsCdnUrl(url));

    [Fact]
    public void BuildsAddressesThatPassItsOwnCheck()
    {
        string?[] built =
        [
            DiscordCdn.ServerIcon("678070694164299796", "f749bb0cbeeb26ef21eca719337d20f1"),
            DiscordCdn.Avatar("268473310986240001", "a_bab14f271d565501444b2ca3be944b25"),
            DiscordCdn.MemberAvatar("1", "2", "abc"),
            DiscordCdn.Emoji("41771983429993937", animated: true),
            DiscordCdn.Sticker("749054660769218631", StickerFormat.Gif),
            DiscordCdn.Sticker("749054660769218631", StickerFormat.Lottie),
        ];

        Assert.All(built, url => Assert.True(DiscordCdn.IsCdnUrl(url), url));
        Assert.Equal("https://cdn.discordapp.com/icons/678070694164299796/f749bb0cbeeb26ef21eca719337d20f1.png", built[0]);
        Assert.Equal("https://cdn.discordapp.com/avatars/268473310986240001/a_bab14f271d565501444b2ca3be944b25.webp?animated=true", built[1]);
        Assert.Equal("https://media.discordapp.net/stickers/749054660769218631.gif", built[4]);
        Assert.Null(DiscordCdn.Avatar("1", null));
    }
}
