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
        let anchors101: Vec<_> =
            roots.iter().map(|cert| webpki101::TrustAnchor::try_from_cert_der(cert).unwrap()).collect();
        let intermediates101: Vec<&[u8]> = chain[1..].iter().map(|cert| cert.as_ref()).collect();
        let result101 = webpki101::EndEntityCert::try_from(chain[0].as_ref()).unwrap().verify_for_usage(
            &[
                &webpki101::ECDSA_P256_SHA256,
                &webpki101::ECDSA_P384_SHA384,
                &webpki101::RSA_PKCS1_2048_8192_SHA256,
                &webpki101::RSA_PKCS1_2048_8192_SHA384,
            ],
            &anchors101,
            &intermediates101,
            webpki101::Time::from_seconds_since_unix_epoch(TIME),
            webpki101::KeyUsage::server_auth(),
            &[],
        );
        let anchors: Vec<_> = roots.iter().map(|cert| webpki102::anchor_from_trusted_cert(cert).unwrap()).collect();
        let time = UnixTime::since_unix_epoch(Duration::from_secs(TIME));
        let result102 = webpki102::EndEntityCert::try_from(&chain[0])
            .unwrap()
            .verify_for_usage(
                webpki102::ALL_VERIFICATION_ALGS,
                &anchors,
                &chain[1..],
                time,
                webpki102::KeyUsage::server_auth(),
                None,
                None,
            )
            .map(|_| ());
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
            assert_eq!(result101, Err(webpki101::Error::CertRevoked));
            assert_eq!(result102, Err(webpki102::Error::CertRevoked));
            assert_eq!(result103, Err(webpki103::Error::CertDistrusted));
        } else {
            assert_eq!(result101, Ok(()));
            assert_eq!(result102, Ok(()));
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
        let emsign = webpki102::anchor_from_trusted_cert(roots.last().unwrap()).unwrap();
        assert!(!roots025::TLS_SERVER_ROOTS.iter().any(|r| r.spki == emsign.subject_public_key_info.as_ref()));
        assert!(!roots1::TLS_SERVER_ROOTS.iter().any(|r| r.subject_public_key_info == emsign.subject_public_key_info));
        assert!(!rootcerts::TLS_SERVER_ROOT_CERTS.iter().any(|r| r == roots.last().unwrap()));
    }
}
