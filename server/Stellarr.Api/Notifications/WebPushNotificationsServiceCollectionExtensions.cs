using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Extensions.Options;

namespace Stellarr.Api.Notifications;

/// <summary>
/// Registers the Web Push options and the only transport that talks to a push
/// service. The operator's keys are validated lazily: an invalid pair makes
/// push unavailable rather than stopping the API at startup.
/// </summary>
public static class WebPushNotificationsServiceCollectionExtensions
{
    public static IServiceCollection AddWebPushNotifications(
        this IServiceCollection services,
        IConfiguration configuration)
    {
        ArgumentNullException.ThrowIfNull(services);
        ArgumentNullException.ThrowIfNull(configuration);

        services.AddOptions<PushOptions>()
            .Bind(configuration.GetSection(PushOptions.SectionName));

        services
            .AddHttpClient(WebPushDelivery.HttpClientName, http =>
            {
                http.Timeout = TimeSpan.FromSeconds(10);
                http.DefaultRequestHeaders.UserAgent.ParseAdd("Stellarr/2.0");
            })
            .ConfigurePrimaryHttpMessageHandler(WebPushDelivery.CreateHandler)
            // A push endpoint contains the browser's capability URL and the
            // VAPID Authorization header carries a signed token; neither may
            // reach the request log.
            .RemoveAllLoggers();

        services.AddSingleton<IWebPushDelivery>(provider => new WebPushDelivery(
            provider.GetRequiredService<IHttpClientFactory>().CreateClient(WebPushDelivery.HttpClientName),
            provider.GetRequiredService<IOptions<PushOptions>>().Value,
            provider.GetService<ILogger<WebPushDelivery>>() ?? NullLogger<WebPushDelivery>.Instance));

        return services;
    }
}
