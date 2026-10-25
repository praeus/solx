use std::path::Path;

use rustls_pki_types::{CertificateDer, ServerName, UnixTime};

use super::*;

const TEST_WRAP_KEY: [u8; 32] = [7u8; 32];

fn file_options() -> CreateOptions {
    CreateOptions {
        backend: BackendKind::File,
        file_protection: FileProtection::Static(TEST_WRAP_KEY),
        ..CreateOptions::default()
    }
}

fn sans(names: &[&str]) -> Vec<San> {
    names.iter().map(|n| n.parse().unwrap()).collect()
}

fn leaf_der(ca: &Authority) -> Vec<u8> {
    pem_to_der(&std::fs::read_to_string(ca.server_cert_path()).unwrap()).unwrap()
}

/// Verify the server certificate the way a TLS client would: chain to the
/// root, server-auth usage, and the name.
fn verify_for(ca: &Authority, name: &str) -> std::result::Result<(), String> {
    let root = CertificateDer::from(ca.root_der().unwrap());
    let anchor = webpki::anchor_from_trusted_cert(&root).map_err(|e| e.to_string())?;
    let leaf = CertificateDer::from(leaf_der(ca));
    let ee = webpki::EndEntityCert::try_from(&leaf).map_err(|e| e.to_string())?;
    ee.verify_for_usage(
        &[webpki::ring::ECDSA_P256_SHA256],
        &[anchor],
        &[],
        UnixTime::now(),
        webpki::KeyUsage::server_auth(),
        None,
        None,
    )
    .map_err(|e| e.to_string())?;
    let server_name = ServerName::try_from(name.to_string()).map_err(|e| e.to_string())?;
    ee.verify_is_valid_for_subject_name(&server_name)
        .map_err(|e| e.to_string())
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap()
}

// ── names and encodings ──────────────────────────────────────────────────────

#[test]
fn sans_parse_ips_and_names() {
    assert_eq!(
        "192.168.1.10".parse::<San>().unwrap(),
        San::Ip("192.168.1.10".parse().unwrap())
    );
    assert_eq!(
        "[::1]".parse::<San>().unwrap(),
        San::Ip("::1".parse().unwrap())
    );
    assert_eq!(
        "MyHost.LAN.".parse::<San>().unwrap(),
        San::Dns("myhost.lan".into())
    );
    assert_eq!(
        "*.example.com".parse::<San>().unwrap(),
        San::Dns("*.example.com".into())
    );
    for bad in [
        "",
        "-bad.example",
        "a..b",
        "spa ce",
        "under_score",
        "http://x",
    ] {
        assert!(bad.parse::<San>().is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn default_sans_cover_loopback() {
    let names: Vec<String> = default_sans().iter().map(San::to_string).collect();
    for expected in ["localhost", "127.0.0.1", "::1"] {
        assert!(names.contains(&expected.to_string()), "{names:?}");
    }
    assert_eq!(normalize_sans(&default_sans()), default_sans());
}

#[test]
fn ecdsa_signatures_are_der_encoded() {
    // High bit set on r: needs a leading zero. Leading zeros on s: trimmed.
    let mut raw = vec![0x80; 32];
    raw.extend([0u8, 0, 0x12].iter().chain([0x34u8; 29].iter()));
    let der = ecdsa_raw_to_der(&raw).unwrap();
    assert_eq!(der[0], 0x30);
    assert_eq!(&der[2..5], &[0x02, 33, 0x00]);
    let s_at = 2 + 2 + 33;
    assert_eq!(&der[s_at..s_at + 3], &[0x02, 30, 0x12]);
    assert_eq!(der[1] as usize, der.len() - 2);
    assert!(ecdsa_raw_to_der(&[1, 2, 3]).is_err());
}

#[test]
fn fingerprints_group_the_pin() {
    let pin = pin_of_spki(b"anything");
    assert_eq!(pin.len(), 43, "base64url of 32 bytes, unpadded");
    let fp = fingerprint_of_pin(&pin);
    assert_eq!(fp.len(), 19);
    assert_eq!(fp.matches('-').count(), 3);
    assert_eq!(fp, fp.to_uppercase());
}

// ── file backend ─────────────────────────────────────────────────────────────

#[test]
fn file_ca_issues_a_server_cert_clients_accept() {
    let dir = tempfile::tempdir().unwrap();
    let mut ca = Authority::create(dir.path(), &file_options()).unwrap();
    assert_eq!(ca.state().backend, BackendKind::File);
    assert_eq!(ca.fingerprint(), fingerprint_of_pin(ca.pin()));

    ca.issue_server(&sans(&["localhost", "127.0.0.1", "solx.lan"]), 397)
        .unwrap();
    for name in ["localhost", "127.0.0.1", "solx.lan"] {
        verify_for(&ca, name).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    assert!(verify_for(&ca, "other.example").is_err());
    assert!(verify_for(&ca, "10.0.0.1").is_err());

    // The chain file is the leaf then the root; the root file is the root.
    let chain = read(dir.path(), state::SERVER_CERT_FILE);
    assert_eq!(chain.matches("BEGIN CERTIFICATE").count(), 2);
    assert!(chain.ends_with(&read(dir.path(), state::ROOT_CERT_FILE)));

    // Keys: the root wrapped, the server key plain PKCS#8 for rustls/OpenSSL.
    assert!(read(dir.path(), state::ROOT_KEY_FILE).contains("SOLX WRAPPED PRIVATE KEY"));
    assert!(read(dir.path(), state::SERVER_KEY_FILE).contains("BEGIN PRIVATE KEY"));
    assert!(!dir.path().join("solx-root.key.new").exists());
    assert!(!dir.path().join("solx-server.key.new").exists());
}

#[test]
fn the_ca_reopens_from_tls_json() {
    let dir = tempfile::tempdir().unwrap();
    let pin = Authority::create(dir.path(), &file_options())
        .unwrap()
        .pin()
        .to_string();
    let mut ca = Authority::open_with(dir.path(), &file_options())
        .unwrap()
        .expect("a CA");
    assert_eq!(ca.pin(), pin);
    ca.issue_server(&sans(&["localhost"]), 30).unwrap();
    verify_for(&ca, "localhost").unwrap();
    assert!(Authority::open(tempfile::tempdir().unwrap().path())
        .unwrap()
        .is_none());
}

#[test]
fn a_second_create_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    Authority::create(dir.path(), &file_options()).unwrap();
    let err = Authority::create(dir.path(), &file_options()).unwrap_err();
    assert!(err.to_string().contains("already has"), "{err}");
}

#[test]
fn a_wrong_wrapping_key_cannot_sign() {
    let dir = tempfile::tempdir().unwrap();
    Authority::create(dir.path(), &file_options()).unwrap();
    let wrong = CreateOptions {
        file_protection: FileProtection::Static([9u8; 32]),
        ..file_options()
    };
    let mut ca = Authority::open_with(dir.path(), &wrong).unwrap().unwrap();
    let err = ca.issue_server(&sans(&["localhost"]), 30).unwrap_err();
    assert!(
        err.to_string().contains("root key is not available"),
        "{err}"
    );
    assert!(ca.state().server.is_none());
}

#[test]
fn plain_protection_writes_a_pkcs8_root_key() {
    let dir = tempfile::tempdir().unwrap();
    let options = CreateOptions {
        file_protection: FileProtection::Plain,
        ..file_options()
    };
    let ca = Authority::create(dir.path(), &options).unwrap();
    assert!(read(dir.path(), state::ROOT_KEY_FILE).contains("BEGIN PRIVATE KEY"));
    assert_eq!(
        ca.state().root.key,
        KeyRef::File {
            path: state::ROOT_KEY_FILE.into(),
            wrapped: false
        }
    );
}

#[test]
fn ensure_server_reissues_only_when_needed() {
    let dir = tempfile::tempdir().unwrap();
    let mut ca = Authority::create(dir.path(), &file_options()).unwrap();
    let names = sans(&["localhost", "127.0.0.1"]);

    assert!(ca.ensure_server(&names).unwrap(), "none yet");
    let first = ca.state().server.clone().unwrap();
    // The same set in another order is the same certificate.
    let reordered = sans(&["127.0.0.1", "localhost", "localhost"]);
    assert!(!ca.ensure_server(&reordered).unwrap());
    assert_eq!(ca.state().server.as_ref().unwrap().sha256, first.sha256);

    // A new name means a new certificate.
    let more = sans(&["localhost", "127.0.0.1", "192.168.1.20"]);
    assert!(ca.ensure_server(&more).unwrap());
    verify_for(&ca, "192.168.1.20").unwrap();

    // Near expiry.
    ca.issue_server(&more, RENEW_WITHIN_DAYS - 1).unwrap();
    assert!(ca.ensure_server(&more).unwrap());
    assert!(ca.state().server.as_ref().unwrap().not_after - Utc::now() > Duration::days(300));

    // Its key went missing.
    std::fs::remove_file(dir.path().join(state::SERVER_KEY_FILE)).unwrap();
    assert!(ca.ensure_server(&more).unwrap());
    verify_for(&ca, "localhost").unwrap();
}

#[test]
fn delete_removes_every_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut ca = Authority::create(dir.path(), &file_options()).unwrap();
    ca.issue_server(&sans(&["localhost"]), 30).unwrap();
    ca.delete().unwrap();
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");
    assert!(Authority::open(dir.path()).unwrap().is_none());
}

#[test]
fn issue_rejects_bad_input() {
    let dir = tempfile::tempdir().unwrap();
    let mut ca = Authority::create(dir.path(), &file_options()).unwrap();
    assert!(ca.issue_server(&[], 30).is_err());
    assert!(ca.issue_server(&sans(&["localhost"]), 0).is_err());
    assert!(ca
        .issue_server(&sans(&["localhost"]), ROOT_VALIDITY_DAYS + 1)
        .is_err());
}

// ── CNG backend ──────────────────────────────────────────────────────────────
//
// These use the real CurrentUser\My store and CNG key storage, in throwaway
// containers named after a temp directory. `Cleanup` deletes the CA even
// when an assertion fails.

#[test]
fn wanted_sans_follow_the_server_config() {
    let dir = tempfile::tempdir().unwrap();
    let config = solx_config::ConfigService::open_in(dir.path()).unwrap();
    config.set("server_host", "10.0.0.5".into()).unwrap();
    config
        .set("server_public_url", "https://solx.example.com".into())
        .unwrap();
    let wanted = wanted_sans(&config);
    for name in [
        "localhost",
        "127.0.0.1",
        "::1",
        "10.0.0.5",
        "solx.example.com",
    ] {
        assert!(
            wanted.contains(&name.parse().unwrap()),
            "{name} in {wanted:?}"
        );
    }
    assert_eq!(certs_dir(&config), dir.path().join("certs"));

    // Bound on every IPv4 interface: no IPv6 addresses but ::1, and never
    // the unspecified address itself.
    config.set("server_host", "0.0.0.0".into()).unwrap();
    config
        .set("server_public_url", serde_json::Value::Null)
        .unwrap();
    let wanted = wanted_sans(&config);
    assert!(!wanted.contains(&San::Ip("0.0.0.0".parse().unwrap())));
    assert!(wanted
        .iter()
        .all(|s| !matches!(s, San::Ip(IpAddr::V6(ip)) if !ip.is_loopback())));
}

#[cfg(all(windows, feature = "cng"))]
mod cng_backend {
    use super::*;
    use crate::cng::CngKey;

    struct Cleanup(std::path::PathBuf);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Ok(Some(ca)) = Authority::open(&self.0) {
                if let Err(e) = ca.delete() {
                    eprintln!("cleanup of {}: {e}", self.0.display());
                }
            }
        }
    }

    fn cng_options(root: Provider, server: Provider) -> CreateOptions {
        CreateOptions {
            backend: BackendKind::Cng,
            root_provider: root,
            server_provider: server,
            file_protection: FileProtection::Plain,
        }
    }

    fn in_my_store(sha256: &str) -> bool {
        let store = schannel::cert_store::CertStore::open_current_user("My").unwrap();
        let found = store.certs().any(|c| {
            c.fingerprint(schannel::cert_context::HashAlgorithm::sha256())
                .map(|f| hex(&f) == sha256)
                .unwrap_or(false)
        });
        found
    }

    fn assert_non_exportable(key: &KeyRef) {
        let key = CngKey::open_for_tests(key).unwrap();
        assert!(!key.private_export_succeeds(), "the private key exported");
    }

    #[test]
    fn software_keys_sign_a_chain_and_never_export() {
        let dir = tempfile::tempdir().unwrap();
        let _cleanup = Cleanup(dir.path().to_path_buf());
        let mut ca = Authority::create(
            dir.path(),
            &cng_options(Provider::Software, Provider::Software),
        )
        .unwrap();
        ca.issue_server(&sans(&["localhost", "127.0.0.1"]), 30)
            .unwrap();
        verify_for(&ca, "localhost").unwrap();
        verify_for(&ca, "127.0.0.1").unwrap();

        let state = ca.state().clone();
        let server = state.server.clone().unwrap();
        let (root_report, server_report) = ca.key_reports();
        let server_report = server_report.unwrap();
        for report in [&root_report, &server_report] {
            assert!(report.present);
            assert!(!report.hardware);
            assert_eq!(report.exportable, Some(false), "{report:?}");
            assert!(report.location.starts_with(crate::cng::SOFTWARE_PROVIDER));
        }
        assert_non_exportable(&state.root.key);
        assert_non_exportable(&server.key);

        // Both certificates are in CurrentUser\My, bound to their keys.
        assert!(in_my_store(&state.root.sha256));
        assert!(in_my_store(&server.sha256));
        // No private key was written to the directory.
        assert!(!dir.path().join(state::ROOT_KEY_FILE).exists());
        assert!(!dir.path().join(state::SERVER_KEY_FILE).exists());

        // Reissuing retires the old certificate and key.
        ca.issue_server(&sans(&["localhost", "solx.lan"]), 30)
            .unwrap();
        assert!(!in_my_store(&server.sha256));
        assert!(CngKey::open_for_tests(&server.key).is_err());
        verify_for(&ca, "solx.lan").unwrap();

        // Reopened from tls.json, the root still signs.
        drop(ca);
        let mut ca = Authority::open(dir.path()).unwrap().unwrap();
        ca.issue_server(&sans(&["localhost"]), 30).unwrap();
        verify_for(&ca, "localhost").unwrap();

        let latest = ca.state().server.clone().unwrap();
        ca.delete().unwrap();
        assert!(!in_my_store(&state.root.sha256));
        assert!(!in_my_store(&latest.sha256));
        assert!(CngKey::open_for_tests(&state.root.key).is_err());
        assert!(CngKey::open_for_tests(&latest.key).is_err());
    }

    #[test]
    fn auto_puts_the_root_on_the_tpm_when_there_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let _cleanup = Cleanup(dir.path().to_path_buf());
        let mut ca =
            Authority::create(dir.path(), &cng_options(Provider::Auto, Provider::Software))
                .unwrap();
        let KeyRef::Cng { provider, .. } = &ca.state().root.key else {
            panic!("not a CNG key");
        };
        let (root_report, _) = ca.key_reports();
        assert_eq!(
            root_report.hardware,
            provider == crate::cng::PLATFORM_PROVIDER
        );
        assert_eq!(root_report.exportable, Some(false));
        eprintln!("root key provider: {provider}");
        assert_non_exportable(&ca.state().root.key.clone());

        ca.issue_server(&sans(&["localhost"]), 30).unwrap();
        verify_for(&ca, "localhost").unwrap();
    }

    #[test]
    fn a_missing_key_is_reported_and_reissued() {
        let dir = tempfile::tempdir().unwrap();
        let _cleanup = Cleanup(dir.path().to_path_buf());
        let mut ca = Authority::create(
            dir.path(),
            &cng_options(Provider::Software, Provider::Software),
        )
        .unwrap();
        let names = sans(&["localhost"]);
        ca.ensure_server(&names).unwrap();
        let server = ca.state().server.clone().unwrap();

        // Someone deleted the key out from under the certificate.
        let backend =
            crate::cng::CngBackend::new(dir.path(), Provider::Software, Provider::Software);
        crate::backend::KeyBackend::delete_key(&backend, &server.key).unwrap();
        assert!(!ca.key_reports().1.unwrap().present);

        assert!(ca.ensure_server(&names).unwrap());
        verify_for(&ca, "localhost").unwrap();
        assert!(!in_my_store(&server.sha256));
    }

    /// TPM against software signing speed, for docs/tls-plan.md. Run with
    /// `cargo test -p solx-certs -- --ignored --nocapture signing_speed`.
    #[test]
    #[ignore]
    fn signing_speed() {
        for provider in [Provider::Tpm, Provider::Software] {
            let dir = tempfile::tempdir().unwrap();
            let _cleanup = Cleanup(dir.path().to_path_buf());
            let started = std::time::Instant::now();
            let ca = match Authority::create(dir.path(), &cng_options(provider, provider)) {
                Ok(ca) => ca,
                Err(e) => {
                    eprintln!("{provider:?}: unavailable ({e})");
                    continue;
                }
            };
            let created = started.elapsed();
            let key = CngKey::open_for_tests(&ca.state().root.key).unwrap();
            let digest = [0x5au8; 32];
            let rounds = 20;
            let started = std::time::Instant::now();
            for _ in 0..rounds {
                key.sign_for_tests(&digest).unwrap();
            }
            let per = started.elapsed() / rounds;
            eprintln!("{provider:?}: create root {created:?}, sign {per:?} each");
        }
    }
}
