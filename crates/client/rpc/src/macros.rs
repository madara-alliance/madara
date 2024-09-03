#[macro_export]
macro_rules! merge_rpc_versions {
    ($rpc_api:expr, $starknet:expr, $read:expr, $write:expr, $trace:expr, $($version:ident),+ $(,)?) => {
        $(
            paste::paste! {
                if $read {
                    $rpc_api.merge(versions::[<$version>]::[<StarknetReadRpcApi $version:upper Server>]::into_rpc($starknet.clone()))?;
                }
                if $write {
                    $rpc_api.merge(versions::[<$version>]::[<StarknetWriteRpcApi $version:upper Server>]::into_rpc($starknet.clone()))?;
                }
                if $trace {
                    $rpc_api.merge(versions::[<$version>]::[<StarknetTraceRpcApi $version:upper Server>]::into_rpc($starknet.clone()))?;
                }
            }
        )+
    };
}
