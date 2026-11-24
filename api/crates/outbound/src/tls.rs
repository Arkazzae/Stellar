//! The TLS configuration: rustls with aws-lc-rs, HTTP/1.1 only, trusting
//! the certificates in the system's bundle file.
//!
//! The bundle is a file and not a call into the operating system so that the
//! same code runs the same way in the container, where `ca-certificates`
//! puts the file, and on a developer's machine. A caller that wants other
//! roots, a private authority for example, builds its own configuration and
//! passes it to the client.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::RootCertStore;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;

use crate::error::{Error, Result};

/// Where the bundle is looked for when `SSL_CERT_FILE` is not set. The
/// first that exists wins. Debian, Ubuntu and Alpine's `ca-certificates`
/// write the first or the second; Red Hat and SUSE the following; macOS
/// ships the second, kept up to date by the system, and Homebrew's
/// `ca-certificates` is the last resort there.
pub const BUNDLE_PATHS: &[&str] = &[
    "/etc/ssl/certs/ca-certificates.crt",
    "/etc/ssl/cert.pem",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/ca-bundle.pem",
    "/etc/pki/tls/cacert.pem",
    "/usr/local/share/certs/ca-root-nss.crt",
    "/opt/homebrew/etc/ca-certificates/cert.pem",
];

/// Reads the trusted roots: the file `SSL_CERT_FILE` names when it is set
/// and not empty, and only that one, so that an operator who points it
/// somewhere gets exactly that set; otherwise the first existing file of
/// [`BUNDLE_PATHS`]. `SSL_CERT_DIR` is not read.
pub fn system_roots() -> Result<RootCertStore> {
    let path = locate_bundle(std::env::var_os("SSL_CERT_FILE"), BUNDLE_PATHS)?;
    roots_from_file(&path)
}

/// Which file to read: the variable's when it is set, else the first of
/// `candidates` that exists.
fn locate_bundle(variable: Option<std::ffi::OsString>, candidates: &[&str]) -> Result<PathBuf> {
    if let Some(path) = variable.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .ok_or_else(|| {
            Error::Roots(
                "no certificate bundle found; install ca-certificates or set SSL_CERT_FILE".into(),
            )
        })
}

/// Reads the certificates of one PEM bundle. A certificate that cannot be
/// parsed is skipped, as bundles are known to hold a few that newer parsers
/// reject; a file with no usable one is an error.
pub fn roots_from_file(path: &Path) -> Result<RootCertStore> {
    let bytes = std::fs::read(path).map_err(|error| {
        Error::Roots(format!(
            "{} cannot be read: {}",
            path.display(),
            error.kind()
        ))
    })?;
    roots_from_pem(&bytes)
}

/// The same as [`roots_from_file`] for bytes already read.
pub fn roots_from_pem(pem: &[u8]) -> Result<RootCertStore> {
    let certificates = CertificateDer::pem_slice_iter(pem).filter_map(|item| item.ok());
    let mut store = RootCertStore::empty();
    let (added, _skipped) = store.add_parsable_certificates(certificates);
    if added == 0 {
        return Err(Error::Roots(
            "the bundle holds no usable certificate".into(),
        ));
    }
    Ok(store)
}

/// A client configuration that trusts `roots`, speaks TLS 1.2 and 1.3 and
/// offers `http/1.1` only.
pub fn client_config(roots: RootCertStore) -> Arc<rustls::ClientConfig> {
    let mut config =
        rustls::ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("the provider supports the default protocol versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Arc::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_with_no_certificate_is_an_error() {
        assert!(roots_from_pem(b"").is_err());
        assert!(roots_from_pem(b"not a certificate").is_err());
        assert!(
            roots_from_pem(b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n")
                .is_err()
        );
    }

    #[test]
    fn the_variable_wins_and_is_not_second_guessed() {
        let file = std::env::current_exe().unwrap();
        let candidate = file.to_str().unwrap();
        let chosen = locate_bundle(Some("/somewhere/else.pem".into()), &[candidate]).unwrap();
        assert_eq!(chosen, PathBuf::from("/somewhere/else.pem"));
    }

    #[test]
    fn an_empty_variable_is_as_good_as_none() {
        let file = std::env::current_exe().unwrap();
        let candidate = file.to_str().unwrap();
        let chosen = locate_bundle(Some("".into()), &["/no/such/file", candidate]).unwrap();
        assert_eq!(chosen, file);
    }

    #[test]
    fn with_no_file_anywhere_the_error_says_what_to_do() {
        let error = locate_bundle(None, &["/no/such/file"]).unwrap_err();
        assert!(error.to_string().contains("SSL_CERT_FILE"));
    }

    #[test]
    fn a_missing_file_is_an_error_that_names_the_path_but_no_secret() {
        let error = roots_from_file(Path::new("/nonexistent/bundle.pem")).unwrap_err();
        assert!(error.to_string().contains("/nonexistent/bundle.pem"));
    }
}
