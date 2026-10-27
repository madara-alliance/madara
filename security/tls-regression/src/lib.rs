#[cfg(test)]
mod tests {
    use rustls_pki_types::{CertificateDer, UnixTime};
    use std::{fs, path::Path, time::Duration};

    // Captured certificates were valid on 2026-10-20. Freeze time so rejection
    // cannot accidentally pass due to fixture expiry in a future CI run.
    const TIME: u64 = 1_792_497_600;

    fn certs(path: &Path) -> Vec<CertificateDer<'static>> {
        rustls_pemfile::certs(&mut fs::read(path).unwrap().as_slice()).collect::<Result<_, _>>().unwrap()
    }

    fn verify_all(chain: &[CertificateDer<'_>], roots: &[CertificateDer<'_>], blocked: bool) {
        let anchors: Vec<_> = roots.iter().map(|cert| webpki103::anchor_from_trusted_cert(cert).unwrap()).collect();
        let time = UnixTime::since_unix_epoch(Duration::from_secs(TIME));
        let result103 = webpki103::EndEntityCert::try_from(&chain[0])
            .unwrap()
            .verify_for_usage(
                webpki103::ALL_VERIFICATION_ALGS,
                &anchors,
                &chain[1..],
                time,
                webpki103::KeyUsage::server_auth(),
                None,
                None,
            )
            .map(|_| ());
        if blocked {
            assert_eq!(result103, Err(webpki103::Error::CertDistrusted));
        } else {
            assert_eq!(result103, Ok(()));
        }
    }

    #[test]
    fn rejects_all_public_chains_even_with_compromised_issuers_explicitly_trusted() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let roots = certs(&fixtures.join("roots.pem"));
        let mut tested = 0;
        for entry in fs::read_dir(&fixtures).unwrap() {
            let path = entry.unwrap().path();
            if !path.file_name().unwrap().to_string_lossy().ends_with(".com.pem") {
                continue;
            }
            let chain = certs(&path);
            verify_all(&chain, &roots, true);
            // A private/custom CA bundle must not bypass the compromised-key ban.
            verify_all(&chain[..1], &chain[1..2], true);
            tested += 1;
        }
        assert_eq!(tested, 12);
    }

    #[test]
    fn preserves_unrelated_private_ca_validation() {
        use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
        let mut ca_params = CertificateParams::new(vec![]).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate().unwrap();
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let leaf_key = KeyPair::generate().unwrap();
        let leaf = CertificateParams::new(vec!["control.example".to_string()])
            .unwrap()
            .signed_by(&leaf_key, &ca, &ca_key)
            .unwrap();
        verify_all(&[leaf.der().clone()], &[ca.der().clone()], false);
    }

    #[test]
    fn removes_emsign_root_from_every_bundled_root_store() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/roots.pem");
        let roots = certs(&fixtures);
        let emsign = webpki103::anchor_from_trusted_cert(roots.last().unwrap()).unwrap();
        assert!(!roots1::TLS_SERVER_ROOTS.iter().any(|r| r.subject_public_key_info == emsign.subject_public_key_info));
        assert!(!rootcerts::TLS_SERVER_ROOT_CERTS.iter().any(|r| r == roots.last().unwrap()));
    }

    #[test]
    fn aws_defaults_select_the_modern_client_even_with_legacy_features_enabled() {
        use aws_smithy_runtime::client::defaults::default_http_client_plugin_v2;
        use aws_smithy_runtime_api::client::behavior_version::BehaviorVersion;
        use aws_smithy_runtime_api::client::http::HttpClient;
        use aws_smithy_runtime_api::client::runtime_components::RuntimeComponentsBuilder;
        use aws_smithy_runtime_api::client::runtime_plugin::RuntimePlugin;

        // Covers the explicit application behavior version and OmniQueue's load_from_env (latest).
        for version in [BehaviorVersion::v2025_08_07(), BehaviorVersion::latest()] {
            let plugin = default_http_client_plugin_v2(version).expect("default HTTPS client enabled");
            let components = plugin.runtime_components(&RuntimeComponentsBuilder::new("regression"));
            let client = components.http_client().unwrap();
            let metadata = client.connector_metadata().unwrap();
            assert_eq!(metadata.name(), "hyper");
            assert_eq!(metadata.version().as_deref(), Some("1.x"));
        }
    }

    #[tokio::test]
    async fn loopback_rpc_clients_reject_tls_urls_instead_of_downgrading() {
        assert!(jsonrpsee_http_client::HttpClientBuilder::default().build("https://127.0.0.1:1").is_err());
        let result = jsonrpsee_ws_client::WsClientBuilder::default().build("wss://127.0.0.1:1").await;
        let error = result.expect_err("WSS must not be accepted without a TLS backend").to_string();
        assert!(error.to_lowercase().contains("tls"), "unexpected rejection: {error}");
        assert!(jsonrpsee_http_client::HttpClientBuilder::default().build("http://127.0.0.1:1").is_ok());
    }
}
