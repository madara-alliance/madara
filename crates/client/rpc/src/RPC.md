# RPC

_This section consists of a brief overview of RPC handling architecture inside
of Madara, as its structure can be quite confusing at first._

## Properties

Madara RPC has the folliwing properties:

**Each RPC category is independent** and decoupled from the rest, so `trace`
methods exist in isolation from `read` methods for example.

**RPC methods are versioned**. It is therefore possible for a user to call
_different versions_ of the same RPC method. This is mostly present for ease of
development of new RPC versions, but also serves to assure a level of backwards
compatibility. To select a specific version of an rpc method, you will need to
append `/rcp/v{version}` to the rpc url you are connecting to.

**RPC versions are grouped under the `Starknet` struct**. This serves as a
common point of implementation for all RPC methods across all versions, and is
also the point of interaction between RPC methods and the node backend.

> [!NOTE]
> All of this is regrouped as an RPC _service_.

## Implementation details

There are **two** main parts to the implementation of RPC methods in Madara.

### Jsonrpsee implementation

> [!NOTE] > `jsonrpsee` is a library developed by Parity which is used to implement JSON
> RPC APIs through simple macro logic.

Each RPC version is defined under the `version` folder using the
`versioned_starknet_rpc` macro. This just serves to rename the trait it is
defined on and all jsonrpsee `#[method]` definitions to include the version
name. The latter is especially important as it avoids name clashes when merging
multiple `RpcModule`s from different versions together.

#### Renaming

```rust
#[versioned_rpc("V0_7_1", "starknet")]
pub trait JsonRpc {
    #[method(name = "blockNumber", and_versions = ["V0_8_0"])]
    fn block_number(&self) -> anyhow::Result<u64>;
}
```

Will become

```rust
#[jsonrpsee::proc_macros::rpc(server, client, namespace = "starknet")]
pub trait JsonRpcV0_7_1 {
    #[method(name = "V0_7_1_blockNumber", aliases = ["starknet_V0_8_0blockNumber"])]
    fn block_number(&self) -> anyhow::Result<u64>;
}
```

### Implementation as a service

> [!IMPORTANT]
> This is where the RPC server is set up and where RPC calls are actually
> parsed, validated, routed and handled.

`RpcService` is responsible for starting the rpc service, and hence the rpc
server. This is done with tower in the following steps:

- RPC apis are built and combined into a single `RpcModule` using
  `versioned_rpc_api`, and all other configurations are loaded.

- Request filtering middleware is set up. This includes host origin filtering
  and CORS filtering.

> [!NOTE]
> Rpc middleware will apply to both websocket and http rpc requests, which is
> why we do not apply versioning in the http middleware.

- Request constraints are set, such as the maximum number of connections and
  request / response size constraints.

- Additional service layers are added on each rpc call inside `service_fn`.
  These are composed into versioning, rate limiting (which is optional) and
  metrics layers. Importantly, version filtering with `RpcMiddlewareServiceVersion`
  will transforms rpc methods request with header `/rpc/v{version}` and a json rpc
  body with a `{method}` field into the correct `starknet_{version}_{method}` rpc
  method call, as this is how we version them internally with jsonrpsee.

> [!NOTE]
> The `starknet` prefix comes from the secondary macro expansion of
> `#[rpc(server, namespace = "starknet)]`

- Finally, the RPC service is added to tower as `RpcServiceBuilder`.
