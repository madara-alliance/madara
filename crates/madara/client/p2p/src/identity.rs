use anyhow::{ensure, Context};
use base64::prelude::*;
use libp2p::{identity::Keypair, PeerId};
use std::{fs, path::Path};

/// This happens to be the pathfinder-compatible identity.json format.
#[derive(serde::Serialize, serde::Deserialize)]
struct P2pIdentity {
    pub private_key: String,
    pub peer_id: String,
}

pub fn load_identity(identity_file: Option<&Path>, save_identity: bool) -> anyhow::Result<Keypair> {
    let Some(identity_file) = identity_file else {
        // no stable identity, generate a new one at every startup.
        return Ok(Keypair::generate_ed25519());
    };

    if save_identity
        && !fs::exists(identity_file).with_context(|| {
            format!("Checking if peer-to-peer identity file at path '{}' exists", identity_file.display())
        })?
    {
        // make & save an peer-to-peer identity file.
        let keypair = Keypair::generate_ed25519();

        let private_key = keypair
            .to_protobuf_encoding()
            .context("Converting newly-created peer-to-peer identity file to protobuf format")?;

        let mut content = serde_json::to_string_pretty(&P2pIdentity {
            private_key: BASE64_STANDARD.encode(private_key),
            peer_id: PeerId::from_public_key(&keypair.public()).to_base58(),
        })
        .context("Converting peer-to-peer identity file to json")?;

        content.push('\n');

        fs::write(identity_file, content)
            .with_context(|| format!("Saving peer-to-peer identity to file at path '{}'", identity_file.display()))?;

        return Ok(keypair);
    }

    let load_identity_file = || {
        let data = fs::read(identity_file).context("Reading file")?;
        let content: P2pIdentity = serde_json::from_slice(&data).context("Parsing json file")?;

        let sk = BASE64_STANDARD.decode(&content.private_key).context("Parsing private_key as base64")?;
        let keypair = Keypair::from_protobuf_encoding(&sk).context("Parsing private_key protobuf encoding")?;

        let peer_id = PeerId::from_public_key(&keypair.public()).to_base58();
        ensure!(
            peer_id == content.peer_id,
            "PeerID derived from secret key does not match with the PeerID in the file. Expected: {peer_id}, Got: {}",
            content.peer_id
        );

        Ok(keypair)
    };

    load_identity_file().with_context(|| {
        format!("Reading and parsing peer-to-peer identity file at path '{}'", identity_file.display())
    })
}
