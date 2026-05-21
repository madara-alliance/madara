#![cfg(test)]

use crate::MadaraBackend;
use mp_chain_config::{ChainConfig, RuntimeExecutionConfig};

#[test]
fn test_clear_runtime_exec_config() {
    let backend = MadaraBackend::open_for_testing(ChainConfig::madara_test().into());
    let chain_config = backend.chain_config();
    let exec_constants = chain_config.exec_constants_by_protocol_version(chain_config.latest_protocol_version).unwrap();
    let runtime_config = RuntimeExecutionConfig::from_current_config(chain_config, exec_constants, false).unwrap();

    backend.write_access().write_runtime_exec_config(&runtime_config).unwrap();
    assert!(backend.get_runtime_exec_config().unwrap().is_some());

    backend.clear_runtime_exec_config().unwrap();

    assert!(backend.get_runtime_exec_config().unwrap().is_none());
}
