using System.Security.Cryptography;
using System.Text;

namespace Stellarr.Api.Auth;

/// <summary>
/// The code the owner account is created with. Until an instance has its
/// first account, whoever opens it first could become its owner; with this
/// code it takes someone who can read the container's log. A new code is drawn
/// at every start of an instance that has no account yet, kept in memory only
/// and never sent by any endpoint, and forgotten once the owner account exists.
/// </summary>
public sealed class SetupCode
{
    // Without 0, 1, I and O, which are easy to mistake for one another when
    // the code is copied by hand.
    private const string Alphabet = "23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    private const int Length = 12;
    private const int GroupLength = 4;

    private readonly Lock _gate = new();
    private byte[]? _code;

    /// <summary>Draws a new code in place of any earlier one, and returns it as a person reads it.</summary>
    public string Issue()
    {
        var code = RandomNumberGenerator.GetString(Alphabet, Length);
        lock (_gate)
        {
            _code = Encoding.ASCII.GetBytes(code);
        }

        return Grouped(code);
    }

    /// <summary>
    /// Whether <paramref name="candidate"/> is the current code. Case, spaces
    /// and dashes do not count, so a code typed as it is read is accepted. The
    /// comparison takes as long whichever character differs.
    /// </summary>
    public bool Matches(string? candidate)
    {
        byte[]? code;
        lock (_gate)
        {
            code = _code;
        }

        if (code is null || candidate is null)
        {
            return false;
        }

        var normalized = new StringBuilder(candidate.Length);
        foreach (var character in candidate)
        {
            if (character != '-' && !char.IsWhiteSpace(character))
            {
                normalized.Append(char.ToUpperInvariant(character));
            }
        }

        return CryptographicOperations.FixedTimeEquals(Encoding.UTF8.GetBytes(normalized.ToString()), code);
    }

    /// <summary>Drops the code once the instance has its owner.</summary>
    public void Forget()
    {
        lock (_gate)
        {
            _code = null;
        }
    }

    /// <summary>
    /// The current code as a person reads it, or null. Only the tests' hosts
    /// read it here; everybody else reads it in the log.
    /// </summary>
    internal string? Current
    {
        get
        {
            lock (_gate)
            {
                return _code is null ? null : Grouped(Encoding.ASCII.GetString(_code));
            }
        }
    }

    private static string Grouped(string code) => string.Join('-', code.Chunk(GroupLength).Select(group => new string(group)));
}
