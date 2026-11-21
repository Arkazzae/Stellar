using System.Runtime.CompilerServices;

namespace Stellarr.Api.Tests;

/// <summary>
/// The suite asserts the Polish texts the server spoke before English became
/// its default, so every host it builds is told to speak Polish. The setting
/// comes in through the environment, under the name an operator uses, because
/// the web application factory is fixed: its own in-memory settings are
/// layered over the environment, which lets a test that wants another language
/// override this one. Every host that speaks Polish has therefore read
/// STELLARR_LANGUAGE through the API's table of variables.
/// </summary>
internal static class TestLanguage
{
    [ModuleInitializer]
    internal static void SelectPolish() => Environment.SetEnvironmentVariable("STELLARR_LANGUAGE", "pl");
}
