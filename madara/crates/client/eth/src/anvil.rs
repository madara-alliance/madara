use alloy::node_bindings::{Anvil, AnvilInstance};
use async_trait::async_trait;
use tokio::sync::OnceCell;

// https://etherscan.io/tx/0xcadb202495cd8adba0d9b382caff907abf755cd42633d23c4988f875f2995d81#eventlog
// The txn we are referring to it is here ^
pub const L1_BLOCK_NUMBER: u64 = 20395662;

#[async_trait]
pub trait AnvilInstanceInitializer: Sync + Send + 'static {
    async fn init() -> AnvilInstance;
}

pub struct SharedAnvil<T: AnvilInstanceInitializer> {
    cell: OnceCell<AnvilInstance>,
    _marker: std::marker::PhantomData<T>,
}

impl<T: AnvilInstanceInitializer> SharedAnvil<T> {
    pub const fn new() -> Self {
        Self { cell: OnceCell::const_new(), _marker: std::marker::PhantomData }
    }

    pub async fn get_instance(&'static self) -> &AnvilInstance {
        self.cell.get_or_init(T::init).await
    }
}

pub struct MainnetFork;

#[async_trait]
impl AnvilInstanceInitializer for MainnetFork {
    async fn init() -> AnvilInstance {
        println!("Initializing Mainnet fork Anvil instance...");
        tokio::task::spawn_blocking(create_anvil_instance).await.expect("Failed to spawn Anvil instance")
    }
}

pub fn create_anvil_instance() -> AnvilInstance {
    let fork_url = std::env::var("ETH_FORK_URL").expect("ETH_FORK_URL not set");
    let anvil = Anvil::new()
        .fork(fork_url)
        .fork_block_number(L1_BLOCK_NUMBER)
        .port(0u16)
        .timeout(480_000)
        .try_spawn()
        .expect("failed to spawn anvil instance");
    println!("Anvil started and running at `{}`", anvil.endpoint());
    anvil
}
