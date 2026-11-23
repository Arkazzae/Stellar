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
    [Fact]
    public void TheChannelMoveCheckSeesEveryChangeTheEvaluationSees()
    {
        OverrideMasks[] states = [default, Allow(Messages), Deny(Messages)];
        const int Owners = 4; // the base role, two held roles, the member
        var slots = new OverrideMasks[Owners, 3];
        var cases = (int)Math.Pow(3, Owners * 3);
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
            foreach (var united in new[] { None, Messages })
            {
                var before = PermissionEvaluator.Evaluate(Member(united, [None, None], At(0), At(2)));
                var after = PermissionEvaluator.Evaluate(Member(united, [None, None], At(1), At(2)));
                if (before != after && !noticed)
                {
                    Assert.Fail($"A move changed case {code} unnoticed.");
                }
            }
        }
    }

    private static PermissionInput Member(ServerPrivileges basePermissions, ServerPrivileges[] held, params OverrideLevel[] levels) =>
        new(false, basePermissions, held, levels);

    private static PermissionInput Owner(params OverrideLevel[] levels) => new(true, None, [], levels);

    private static OverrideLevel Level(
        OverrideMasks baseMasks = default, OverrideMasks[]? roles = null, OverrideMasks member = default) =>
        new(baseMasks, roles ?? [], member);

    private static OverrideMasks Allow(ServerPrivileges bits) => new(bits, None);

    private static OverrideMasks Deny(ServerPrivileges bits) => new(None, bits);
}
