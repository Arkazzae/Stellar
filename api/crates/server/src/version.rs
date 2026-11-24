//! The version of this server, in one place.
//!
//! It is what the image was built as: the build says it in the environment
//! variable `STELLARR_VERSION` when it compiles the server (the container build
//! passes its `VERSION` argument), and without one it is the version of the
//! workspace in `api/Cargo.toml`. Whatever follows a plus sign, a revision
//! such as `1.4.0+3f9c2e1`, is the build's and is not part of the version the
//! server reports.
//!
//! Nothing else reads `STELLARR_VERSION` or `CARGO_PKG_VERSION`: a part of the
//! server that tells the version (the discovery document today) calls
//! [`server`].

/// The version as it was built, revision included.
const BUILT_AS: &str = match option_env!("STELLARR_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The version this server reports, without the revision.
pub fn server() -> &'static str {
    BUILT_AS.split('+').next().unwrap_or(BUILT_AS)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_version_has_no_revision_and_is_not_empty() {
        assert!(!super::server().is_empty());
        assert!(!super::server().contains('+'));
    }
}
