namespace Stellarr.Api.Domain;

/// <summary>
/// What the instance owner decided in Cockpit about one configured identity
/// provider, keyed by the provider's id. The provider itself is configured
/// in the environment; a provider without a row signs people in and creates
/// no accounts.
/// </summary>
public sealed class IdentityProviderSetting
{
    public required string ProviderId { get; set; }

    /// <summary>
    /// The provider may create an account, with no password, for someone it
    /// confirmed who also has an invite. Off unless the owner turns it on.
    /// </summary>
    public bool SignUpAllowed { get; set; }

    public DateTime UpdatedAt { get; set; }
}
