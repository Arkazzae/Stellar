using Stellarr.Api.Auth;
using Stellarr.Api.Domain;

namespace Stellarr.Api.Tests;

/// <summary>
/// The evaluation order on its own, without a database: the base role, the
/// held roles together, the member, level by level from the section inward.
/// Each case is named, so a failure says which rule broke.
/// </summary>
public sealed class PermissionEvaluatorTests
{
    private const ServerPrivileges None = ServerPrivileges.None;
    private const ServerPrivileges Messages = ServerPrivileges.ManageMessages;
    private const ServerPrivileges Bans = ServerPrivileges.ManageBans;
    private const ServerPrivileges Channels = ServerPrivileges.ManageChannels;
    private const ServerPrivileges Audit = ServerPrivileges.ViewAuditLog;
    private const ServerPrivileges Admin = ServerPrivileges.Administrator;
    private const ServerPrivileges View = ServerPrivileges.ViewChannel;
    private const ServerPrivileges Send = ServerPrivileges.SendMessages;

    private static readonly Dictionary<string, (PermissionInput Input, ServerPrivileges Expected)> Cases = new()
    {
        // Server-wide: the base role united with every held role.
        ["base role alone"] = (Member(Messages, []), Messages),
        ["held roles unite with the base role"] = (Member(Audit, [Messages, Bans]), Audit | Messages | Bans),
        ["no roles and an empty base role hold nothing"] = (Member(None, []), None),

        // The owner skips every override and holds bits no role grants.
        ["owner holds everything"] = (
            Owner(Level(baseMasks: Deny(ServerPrivileges.All), roles: [Deny(ServerPrivileges.All)], member: Deny(ServerPrivileges.All))),
            ServerPrivileges.All),

        // Administrator, from any role, holds everything and passes every
        // override; an override can neither grant it nor take it away.
        ["administrator passes every override"] = (
            Member(None, [Admin], Level(baseMasks: Deny(ServerPrivileges.All), roles: [Deny(ServerPrivileges.All)],
                member: Deny(ServerPrivileges.All))),
            ServerPrivileges.All),
        ["administrator through the base role"] = (Member(Admin, [], Level(member: Deny(Messages))), ServerPrivileges.All),
        ["no override grants administrator"] = (
            Member(None, [None], Level(baseMasks: Allow(Admin | Bans), roles: [Allow(Admin)], member: Allow(Admin)),
                Level(baseMasks: Allow(Admin))),
            Bans),
        ["a deny of administrator in an override is ignored too"] = (
            Member(Messages, [None], Level(baseMasks: Deny(Admin), roles: [Deny(Admin)], member: Deny(Admin))), Messages),

        // One level, one source at a time.
        ["base override denies"] = (Member(Messages | Bans, [], Level(baseMasks: Deny(Messages))), Bans),
        ["base override allows"] = (Member(None, [], Level(baseMasks: Allow(Bans))), Bans),
        ["held role override denies what the role grants"] = (Member(None, [Messages], Level(roles: [Deny(Messages)])), None),
        ["held role override allows what no role grants"] = (Member(None, [None], Level(roles: [Allow(Channels)])), Channels),
        ["member override denies"] = (Member(None, [Messages], Level(member: Deny(Messages))), None),
        ["member override allows"] = (Member(None, [], Level(member: Allow(Bans))), Bans),

        // Across roles an allow beats a deny, whichever role ranks higher.
        ["allow of one role beats deny of another"] = (
            Member(Messages, [None, None], Level(roles: [Deny(Messages), Allow(Messages)])), Messages),
        ["allow beats deny in either order"] = (
            Member(Messages, [None, None], Level(roles: [Allow(Messages), Deny(Messages)])), Messages),
        ["deny of one role stands when no role allows"] = (
            Member(Messages | Bans, [None, None], Level(roles: [Deny(Messages), Allow(Bans)])), Bans),

        // Within a level the held roles come after the base role and before the member.
        ["held role deny beats base allow"] = (Member(None, [None], Level(baseMasks: Allow(Bans), roles: [Deny(Bans)])), None),
        ["held role allow beats base deny"] = (Member(Bans, [None], Level(baseMasks: Deny(Bans), roles: [Allow(Bans)])), Bans),
        ["member deny beats role allow"] = (Member(None, [None], Level(roles: [Allow(Bans)], member: Deny(Bans))), None),
        ["member allow beats role deny"] = (Member(Bans, [None], Level(roles: [Deny(Bans)], member: Allow(Bans))), Bans),

        // Inside one override a bit both allowed and denied stays denied.
        ["one override allowing and denying a bit denies it"] = (
            Member(None, [], Level(baseMasks: new OverrideMasks(Bans | Audit, Bans))), Audit),
        ["one role allowing and denying a bit does not beat another role's deny"] = (
            Member(Bans, [None, None], Level(roles: [new OverrideMasks(Bans, Bans), Deny(Bans)])), None),

        // Levels layer from the section inward; the channel has the last word.
        ["channel allow beats section deny"] = (
            Member(Messages, [], Level(baseMasks: Deny(Messages)), Level(baseMasks: Allow(Messages))), Messages),
        ["channel deny beats section allow"] = (
            Member(None, [None], Level(roles: [Allow(Messages)]), Level(roles: [Deny(Messages)])), None),
        ["section decision stands where the channel is silent"] = (
            Member(None, [None], Level(roles: [Allow(Channels)]), Level(roles: [Deny(Messages)])), Channels),
        ["base allow at the channel beats a role deny at the section"] = (
            Member(Messages, [None], Level(roles: [Deny(Messages)]), Level(baseMasks: Allow(Messages))), Messages),
        ["role deny at the channel beats a member allow at the section"] = (
            Member(None, [None], Level(member: Allow(Bans)), Level(roles: [Deny(Bans)])), None),
        ["channel outside a section has its own level only"] = (
            Member(Messages, [None], Level(roles: [Deny(Messages)])), None),

        // In a channel, without view there is nothing; elsewhere the levels decide alone.
        ["in a channel, view and the rest"] = (InChannel(View | Send, [], Level()), View | Send),
        ["in a channel, without view nothing is held"] = (InChannel(Send | Messages, [Bans], Level()), None),
        ["the base role denied view in the channel holds nothing there"] = (
            InChannel(View | Send, [Messages], Level(baseMasks: Deny(View))), None),
        ["view denied at the section reaches its channels"] = (
            InChannel(View | Send, [], Level(baseMasks: Deny(View)), Level()), None),
        ["the channel allowing view beats the section denying it"] = (
            InChannel(View | Send, [], Level(baseMasks: Deny(View)), Level(baseMasks: Allow(View))), View | Send),
        ["a held role's allow of view beats the base role's deny"] = (
            InChannel(View | Send, [None], Level(baseMasks: Deny(View), roles: [Allow(View)])), View | Send),
        ["a member's own deny of view hides the channel"] = (
            InChannel(View | Send, [None], Level(roles: [Allow(View)], member: Deny(View))), None),
        ["a member's own allow of view shows it"] = (
            InChannel(Send, [], Level(baseMasks: Deny(View), member: Allow(View))), View | Send),
        ["a base role without view sees no channel"] = (InChannel(Send, [], Level()), None),
        ["an override can still grant what a channel shows"] = (
            InChannel(None, [], Level(baseMasks: Allow(View | Messages))), View | Messages),
        ["the owner views every channel"] = (
            new PermissionInput(true, None, [], [Level(baseMasks: Deny(View))], AtChannel: true), ServerPrivileges.All),
        ["an administrator views every channel"] = (
            InChannel(None, [Admin], Level(baseMasks: Deny(View), roles: [Deny(View)], member: Deny(View))),
            ServerPrivileges.All),
        ["at a section, without view what it allows still holds"] = (
            Member(Channels, [], Level(baseMasks: Deny(View))), Channels),
    };

    public static TheoryData<string> Names => new(Cases.Keys);

    [Theory]
    [MemberData(nameof(Names))]
    public void EvaluatesInDiscordsOrder(string name)
    {
        var (input, expected) = Cases[name];

        Assert.Equal(expected, PermissionEvaluator.Evaluate(input));
    }

    [Fact]
    public void NoLevelsGiveTheServerWidePermissions()
    {
        var input = Member(Audit, [Messages], Level(roles: [Deny(Messages)]));

        Assert.Equal(Audit | Messages, PermissionEvaluator.Evaluate(input with { Levels = [] }));
        Assert.Equal(Audit | Messages, PermissionEvaluator.ServerWide(false, Audit, [Messages]));
        Assert.Equal(ServerPrivileges.All, PermissionEvaluator.ServerWide(true, None, []));
        Assert.Equal(ServerPrivileges.All, PermissionEvaluator.ServerWide(false, None, [Admin]));
    }

    /// <summary>
    /// The channel move check asks each role and member alone whether its
    /// overrides could change a bit, instead of evaluating everybody. Every
    /// combination of one bit, for the base role, two held roles and the
    /// member, at both sections and the channel, proves that it never misses
    /// a change the evaluation sees.
    /// </summary>
    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void TheChannelMoveCheckSeesEveryChangeTheEvaluationSees(bool view)
    {
        // The bit a move may change: managing messages, evaluated as layering
        // alone, or view, evaluated in the channel, where losing it takes
        // everything else with it.
        var bit = view ? View : Messages;
        OverrideMasks[] states = [default, Allow(bit), Deny(bit)];
        const int Owners = 4; // the base role, two held roles, the member
        var slots = new OverrideMasks[Owners, 3];
        var cases = (int)Math.Pow(3, Owners * 3);
        var changes = 0;
        for (var code = 0; code < cases; code++)
        {
            var rest = code;
            for (var owner = 0; owner < Owners; owner++)
            {
                for (var place = 0; place < 3; place++)
                {
                    slots[owner, place] = states[rest % 3];
                    rest /= 3;
                }
            }

            OverrideLevel At(int place) => new(slots[0, place], [slots[1, place], slots[2, place]], slots[3, place]);
            var noticed = Enumerable.Range(0, Owners).Any(owner =>
                ServerAuthorization.Changed(slots[owner, 0], slots[owner, 1], slots[owner, 2]) != None);
            // With view, the member holds sending too, so a whole result that
            // changes shows whether view, and with it everything, came or went.
            var others = view ? Send : None;
            foreach (var united in new[] { others, bit | others })
            {
                var before = PermissionEvaluator.Evaluate(new PermissionInput(false, united, [None, None], [At(0), At(2)], view));
                var after = PermissionEvaluator.Evaluate(new PermissionInput(false, united, [None, None], [At(1), At(2)], view));
                if (before != after && !noticed)
                {
                    Assert.Fail($"A move changed case {code} unnoticed.");
                }

                if (before != after)
                {
                    changes++;
                }
            }
        }

        // Neither run may pass for want of a move that changes anything.
        Assert.True(changes > 0);
    }

    /// <summary>A member somewhere the view rule does not reach: the server, or a section.</summary>
    private static PermissionInput Member(ServerPrivileges basePermissions, ServerPrivileges[] held, params OverrideLevel[] levels) =>
        new(false, basePermissions, held, levels, AtChannel: false);

    /// <summary>A member in a channel, where without view nothing is held.</summary>
    private static PermissionInput InChannel(ServerPrivileges basePermissions, ServerPrivileges[] held, params OverrideLevel[] levels) =>
        new(false, basePermissions, held, levels, AtChannel: true);

    private static PermissionInput Owner(params OverrideLevel[] levels) => new(true, None, [], levels, AtChannel: false);

    private static OverrideLevel Level(
        OverrideMasks baseMasks = default, OverrideMasks[]? roles = null, OverrideMasks member = default) =>
        new(baseMasks, roles ?? [], member);

    private static OverrideMasks Allow(ServerPrivileges bits) => new(bits, None);

    private static OverrideMasks Deny(ServerPrivileges bits) => new(None, bits);
}
