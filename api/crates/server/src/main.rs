//! The `stellarr-api` binary: settings from the environment, logging, and the
//! exit status.

use std::process::ExitCode;

use stellarr_api::settings::Settings;

fn main() -> ExitCode {
    let variables: Vec<(String, String)> = std::env::vars().collect();
    let settings = match Settings::load(&variables) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("stellarr-api: {error}");
            return ExitCode::FAILURE;
        }
    };
    let log = stellarr_api::logging::LogBuffer::new();
    if let Err(error) = stellarr_api::logging::init(&settings.log, vec![log.sink()]) {
        eprintln!("stellarr-api: {error}");
        return ExitCode::FAILURE;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "cannot start the runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(stellarr_api::run(settings, log)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "stellarr-api stopped");
            ExitCode::FAILURE
        }
    }
}
