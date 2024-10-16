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
compatibility. The syntax for selecting a _versioned_ method is
`/rcp/v{version}/{method}` during an RPC call.

**RPC versions are grouped under the `Starknet` struct**. This serves as a
common point of implementation for all RPC methods across all versions, and is
also the point of interaction between RPC methods and the node backend.

> [!NOTE]
> All of this is regrouped as an RPC _service_.

## Implementation details

There are **two** main parts to the implementation of RPC methods in Madara.

### Jsonrpsee implementation

> [!NOTE]
> `jsonrpsee` is a library developed by Parity which is used to implement JSON
> RPC APIs through simple macro logic.

Each RPC version is defined under the `version` folder using the
`versioned_starknet_rpc` macro. This just serves to rename the trait it is
defined on and all jsonrpsee `#[method]` definitions to include the version
name. The latter is especially important as it avoids name clashes when merging
multiple `RpcModule`s from different versions together.

#### Renaming

```rust
#[versioned_starknet_rpc("V0_7_1)")]
trait yourTrait {
  #[method(name = "foo")]
  async fn foo();
}
```

Will become

```rust
// This is actually `jsonrpsee::proc_macros::rpc`, but the
// `versioned_starknet_rpc` macro is not particularly hygenic
#[rpc(server, namespace = "starknet")]
trait yourTraitV0_7_1 {
  #[method(name = "V0_7_1_foo")]
  async fn foo();
}
```

### Implementation as a service

> [!IMPORTANT]
> This is where the RPC server is set up and where RPC calls are actually
> parsed, validated, routed and handled.

`RpcService` is responsible for starting the rpc service, and hence the rpc
server. This is done with tower in the following steps:

1. RPC apis are built and combined into a single `RpcModule` using
`versioned_rpc_api`, and all other configurations are loaded.

2. Request filtering middleware is set up. This includes host origin filtering
and CORS filtering, but most importantly version filtering with
`VersionMiddlewareLayer`, which transforms rpc methods request under the format
`/rpc/v{version}/{method}` into the correct `starknet_{version}_{method}`

> [!NOTE]
> The `starknet` prefix comes from the secondary macro expansion of
> `#[rpc(server, namespace = "starknet)]`

2. Request constraints are set, such as the maximum number of connections and
request / response size constraints.

3. Metrics (optional) and rate limiting are enforced with `MiddlewareLayer`

4. Finally, the RPC service is added to tower as `RpcServiceBuilder`. Note that
the actual RPC selection logic is handled behind the hood by `jsonrpsee`.
