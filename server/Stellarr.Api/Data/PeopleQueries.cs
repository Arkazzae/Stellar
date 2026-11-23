using Stellarr.Api.Domain;

namespace Stellarr.Api.Data;

public static class PeopleQueries
{
    /// <summary>
    /// The accounts of people, without the placeholders that stand in for the
    /// authors of imported messages. Every lookup that finds somebody by an id
    /// or a name somebody else typed goes through this, so a placeholder is
    /// answered exactly as an account that does not exist.
    /// </summary>
    public static IQueryable<User> People(this IQueryable<User> users) => users.Where(x => x.Origin == null);
}
