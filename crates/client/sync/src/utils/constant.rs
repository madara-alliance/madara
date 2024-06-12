//! Deoxys constants.
// Starknet Core contract addresses on Ethereum.
//
// These addresses are used to verified the L2 state accross the L1.
pub mod starknet_core_address {
    pub const MAINNET: &str = "0xc662c410C0ECf747543f5bA90660f6ABeBD9C8c4";
    pub const GOERLI_TESTNET: &str = "0xde29d060D45901Fb19ED6C6e959EB22d8626708e";
    pub const GOERLI_INTEGRATION: &str = "0xd5c325D183C592C94998000C5e0EED9e6655c020";
    pub const SEPOLIA_TESTNET: &str = "0xE2Bb56ee936fd6433DC0F6e7e3b8365C906AA057";
    pub const SEPOLIA_INTEGRATION: &str = "0x4737c0c1B4D5b1A687B42610DdabEE781152359c";
}

pub const LOG_STATE_UPDTATE_TOPIC: &str = "0xd342ddf7a308dec111745b00315c14b7efb2bdae570a6856e088ed0c65a3576c";

// This a list of free RPC URLs for Ethereum mainnet.
//
// It is used whenever no --l1_endpoint is provided.
// It should be used only for testing purposes.
//
// The list comes from DefiLlama's chainlist repository:
// https://github.com/DefiLlama/chainlist
pub const L1_FREE_RPC_URLS: &[&str] = &[
    "https://endpoints.omniatech.io/v1/eth/mainnet/public",
    "https://rpc.ankr.com/eth",
    "https://go.getblock.io/aefd01aa907c4805ba3c00a9e5b48c6b",
    "https://eth-mainnet.nodereal.io/v1/1659dfb40aa24bbb8153a677b98064d7",
    "https://ethereum-rpc.publicnode.com",
    "wss://ethereum-rpc.publicnode.com",
    "https://1rpc.io/eth",
    "https://rpc.builder0x69.io/",
    "https://rpc.mevblocker.io",
    "https://rpc.flashbots.net/",
    "https://cloudflare-eth.com/",
    "https://eth-mainnet.public.blastapi.io",
    "https://api.securerpc.com/v1",
    "https://openapi.bitstack.com/v1/wNFxbiJyQsSeLrX8RRCHi7NpRxrlErZk/DjShIqLishPCTB9HiMkPHXjUM9CNM9Na/ETH/mainnet",
    "https://eth-pokt.nodies.app",
    "https://eth-mainnet-public.unifra.io",
    "https://ethereum.blockpi.network/v1/rpc/public",
    "https://rpc.payload.de",
    "https://api.zmok.io/mainnet/oaen6dy8ff6hju9k",
    "https://eth-mainnet.g.alchemy.com/v2/demo",
    "https://eth.api.onfinality.io/public",
    "https://core.gashawk.io/rpc",
    "https://mainnet.eth.cloud.ava.do/",
    "https://ethereumnodelight.app.runonflux.io",
    "https://eth-mainnet.rpcfast.com?api_key=xbhWBI1Wkguk8SNMu1bvvLurPGLXmgwYeC4S6g2H7WdwFigZSmPWVZRxrskEQwIf",
    "https://main-light.eth.linkpool.io",
    "https://rpc.eth.gateway.fm",
    "https://rpc.chain49.com/ethereum?api_key=14d1a8b86d8a4b4797938332394203dc",
    "https://eth.meowrpc.com",
    "https://eth.drpc.org",
    "https://api.zan.top/node/v1/eth/mainnet/public",
    "https://eth-mainnet.diamondswap.org/rpc",
    "https://rpc.notadegen.com/eth",
    "https://eth.merkle.io",
    "https://rpc.lokibuilder.xyz/wallet",
    "https://services.tokenview.io/vipapi/nodeservice/eth?apikey=qVHq2o6jpaakcw3lRstl",
    "https://eth.nodeconnect.org/",
    "https://api.stateless.solutions/ethereum/v1/demo",
    "https://rpc.polysplit.cloud/v1/chain/1",
    "https://public.stackup.sh/api/v1/node/ethereum-mainnet",
    "https://api.tatum.io/v3/blockchain/node/ethereum-mainnet",
    "https://eth.nownodes.io",
    "https://rpc.nodifi.ai/api/rpc/free",
    "https://ethereum.rpc.subquery.network/public",
    "https://rpc.graffiti.farm",
    "https://rpc.public.curie.radiumblock.co/http/ethereum",
];
