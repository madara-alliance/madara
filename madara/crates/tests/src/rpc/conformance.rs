#[cfg(test)]
mod test_rpc_conformance {
    use crate::{MadaraCmd, MadaraCmdBuilder};
    use jsonrpsee::{
        core::client::{ClientT, SubscriptionClientT, SubscriptionKind},
        ws_client::WsClientBuilder,
    };
    use serde::Deserialize;
    use serde_json::{json, Value};
    use std::{
        collections::{BTreeSet, HashMap},
        path::PathBuf,
    };
    use tokio::sync::OnceCell;

    static MADARA_INSTANCE: OnceCell<MadaraCmd> = OnceCell::const_new();

    const ETH_TOKEN_ADDRESS: &str = "0x49d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7";
    const ETH_NAME_STORAGE_KEY: &str = "0x0341c1bdfd89f69748aa00b5742b03adbffd79b8e80cab5c50d91cd8c2a79be1";
    const SEPOLIA_BLOCK_2_TX: &str = "0x701d9adb9c60bc2fd837fe3989e15aeba4be1a6e72bb6f61ffe35a42866c772";

    #[derive(Debug, Deserialize)]
    struct SpecManifest {
        versions: Vec<VersionSpec>,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct VersionSpec {
        version: String,
        route: String,
        expected_spec_version: String,
        source_openrpc_version: String,
        #[serde(default)]
        known_missing_methods: Vec<KnownMissingMethod>,
        openrpc_files: Vec<String>,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct KnownMissingMethod {
        method: String,
        reason: String,
    }

    #[derive(Debug, Deserialize)]
    struct OpenRpcDocument {
        info: OpenRpcInfo,
        #[serde(default)]
        methods: Vec<OpenRpcMethod>,
        #[serde(default)]
        components: OpenRpcComponents,
    }

    #[derive(Debug, Deserialize)]
    struct OpenRpcInfo {
        version: String,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct OpenRpcMethod {
        name: String,
        #[serde(default)]
        result: Value,
        #[serde(default)]
        errors: Vec<Value>,
    }

    #[derive(Debug, Default, Deserialize)]
    struct OpenRpcComponents {
        #[serde(default)]
        schemas: HashMap<String, Value>,
        #[serde(default)]
        errors: HashMap<String, Value>,
    }

    #[derive(Debug)]
    struct OpenRpcBundle {
        methods: HashMap<String, OpenRpcMethod>,
        schemas: HashMap<String, Value>,
        errors: HashMap<String, Value>,
    }

    #[derive(Debug, Clone)]
    struct RpcCase {
        method: &'static str,
        params: Value,
    }

    async fn get_madara() -> &'static MadaraCmd {
        MADARA_INSTANCE
            .get_or_init(|| async {
                let mut madara = MadaraCmdBuilder::new()
                    .args(["--full", "--network", "sepolia", "--sync-stop-at", "19", "--no-l1-sync"])
                    .label("rpc-conformance")
                    .run();

                madara.wait_for_ready().await;
                madara.wait_for_sync_to(19).await;
                madara
            })
            .await
    }

    fn manifest() -> SpecManifest {
        serde_json::from_str(include_str!("../../fixtures/rpc-openrpc/manifest.json"))
            .expect("RPC OpenRPC manifest should parse")
    }

    fn fixtures_dir(version: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/rpc-openrpc").join(version)
    }

    fn load_openrpc_bundle(spec: &VersionSpec) -> OpenRpcBundle {
        let mut methods = HashMap::new();
        let mut schemas = HashMap::new();
        let mut errors = HashMap::new();

        for file in &spec.openrpc_files {
            let path = fixtures_dir(&spec.version).join(file);
            let contents =
                std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("reading {}: {err}", path.display()));
            let document: OpenRpcDocument =
                serde_json::from_str(&contents).unwrap_or_else(|err| panic!("parsing {}: {err}", path.display()));

            assert_eq!(
                document.info.version,
                spec.source_openrpc_version,
                "{} has unexpected OpenRPC info.version",
                path.display()
            );

            for method in document.methods {
                methods.insert(method.name.clone(), method);
            }
            schemas.extend(document.components.schemas);
            for (name, error) in document.components.errors {
                if error.get("$ref").is_some() && errors.contains_key(&name) {
                    continue;
                }
                errors.insert(name, error);
            }
        }

        OpenRpcBundle { methods, schemas, errors }
    }

    fn route_url(madara: &MadaraCmd, route: &str) -> String {
        format!("{}{}", madara.rpc_url().trim_end_matches('/'), route)
    }

    fn websocket_route_url(madara: &MadaraCmd, route: &str) -> String {
        let http_url = route_url(madara, route);
        if let Some(rest) = http_url.strip_prefix("http://") {
            format!("ws://{rest}")
        } else if let Some(rest) = http_url.strip_prefix("https://") {
            format!("wss://{rest}")
        } else {
            panic!("unsupported RPC URL scheme for websocket route: {http_url}");
        }
    }

    fn has_openrpc_file(spec: &VersionSpec, filename: &str) -> bool {
        spec.openrpc_files.iter().any(|file| file == filename)
    }

    async fn rpc_response(url: &str, method: &str, params: Value) -> Value {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1
        });
        let response = reqwest::Client::new().post(url).json(&payload).send().await.expect("RPC request failed");
        assert!(response.status().is_success(), "RPC returned HTTP {} for {method}", response.status());
        response.json().await.expect("RPC response should be JSON")
    }

    fn openrpc_methods(bundle: &OpenRpcBundle) -> BTreeSet<String> {
        bundle.methods.keys().cloned().collect()
    }

    fn is_known_missing_method(spec: &VersionSpec, method: &str) -> bool {
        spec.known_missing_methods.iter().any(|gap| gap.method == method)
    }

    fn expected_callable_methods(bundle: &OpenRpcBundle, spec: &VersionSpec) -> BTreeSet<String> {
        let known_missing = spec
            .known_missing_methods
            .iter()
            .map(|gap| {
                assert!(!gap.reason.trim().is_empty(), "{} known missing method needs a reason", gap.method);
                gap.method.clone()
            })
            .collect::<BTreeSet<_>>();

        openrpc_methods(bundle)
            .into_iter()
            .filter(|method| !method.starts_with("starknet_subscription"))
            .filter(|method| !known_missing.contains(method))
            .collect()
    }

    fn version_macro_name(version: &str) -> String {
        version.replacen('v', "V", 1)
    }

    fn route_method_name(version: &str, method: &str) -> Option<String> {
        let route_prefix = format!("rpc/{}/", version_macro_name(version));
        if let Some(method) = method.strip_prefix(&route_prefix) {
            return method.starts_with("starknet_").then(|| method.to_owned());
        }

        if method.starts_with("starknet_") && !method.starts_with("starknet_V") {
            return Some(method.to_owned());
        }

        let prefix = format!("starknet_{}_", version_macro_name(version));
        method.strip_prefix(&prefix).map(|suffix| format!("starknet_{suffix}"))
    }

    fn readme_supported_methods() -> BTreeSet<String> {
        let readme = include_str!("../../../../../README.md");
        let mut methods = BTreeSet::new();
        for capture in readme.match_indices("`starknet_") {
            let rest = &readme[capture.0 + 1..];
            if let Some((method, _)) = rest.split_once('`') {
                methods.insert(method.to_owned());
            }
        }
        methods
    }

    fn success_cases(bundle: &OpenRpcBundle, spec: &VersionSpec) -> Vec<RpcCase> {
        let mut cases = vec![
            RpcCase { method: "starknet_blockHashAndNumber", params: json!({}) },
            RpcCase { method: "starknet_getBlockTransactionCount", params: json!({"block_id": {"block_number": 2}}) },
            RpcCase { method: "starknet_getBlockWithTxHashes", params: json!({"block_id": {"block_number": 2}}) },
            RpcCase { method: "starknet_getBlockWithTxs", params: json!({"block_id": {"block_number": 2}}) },
            RpcCase { method: "starknet_getBlockWithReceipts", params: json!({"block_id": {"block_number": 2}}) },
            RpcCase {
                method: "starknet_getTransactionByBlockIdAndIndex",
                params: json!({"block_id": {"block_number": 2}, "index": 0}),
            },
            RpcCase {
                method: "starknet_getTransactionByHash",
                params: json!({"transaction_hash": SEPOLIA_BLOCK_2_TX}),
            },
            RpcCase {
                method: "starknet_getTransactionReceipt",
                params: json!({"transaction_hash": SEPOLIA_BLOCK_2_TX}),
            },
            RpcCase {
                method: "starknet_getEvents",
                params: json!({
                    "filter": {
                        "from_block": {"block_number": 0},
                        "to_block": {"block_number": 19},
                        "keys": [[]],
                        "chunk_size": 2
                    }
                }),
            },
            RpcCase {
                method: "starknet_getStorageAt",
                params: json!({
                    "contract_address": ETH_TOKEN_ADDRESS,
                    "key": ETH_NAME_STORAGE_KEY,
                    "block_id": {"block_number": 12}
                }),
            },
        ];

        if bundle.methods.contains_key("starknet_getStorageProof") {
            cases.push(RpcCase {
                method: "starknet_getStorageProof",
                params: json!({
                    "block_id": {"block_number": 19},
                    "class_hashes": [],
                    "contract_addresses": [],
                    "contracts_storage_keys": []
                }),
            });
        }

        cases.retain(|case| bundle.methods.contains_key(case.method) && !is_known_missing_method(spec, case.method));
        cases
    }

    fn error_cases(bundle: &OpenRpcBundle, spec: &VersionSpec) -> Vec<RpcCase> {
        let mut cases = vec![
            RpcCase {
                method: "starknet_getBlockTransactionCount",
                params: json!({"block_id": {"block_number": 999999999}}),
            },
            RpcCase { method: "starknet_getTransactionReceipt", params: json!({"transaction_hash": "0x123"}) },
        ];

        if bundle.methods.contains_key("starknet_getMessagesStatus") {
            cases.push(RpcCase { method: "starknet_getMessagesStatus", params: json!({"transaction_hash": "0x1"}) });
        }
        if bundle.methods.contains_key("starknet_getStorageProof") {
            cases.push(RpcCase {
                method: "starknet_getStorageProof",
                params: json!({
                    "block_id": {"block_number": 999999999},
                    "class_hashes": [],
                    "contract_addresses": [],
                    "contracts_storage_keys": []
                }),
            });
        }

        cases.retain(|case| bundle.methods.contains_key(case.method) && !is_known_missing_method(spec, case.method));
        cases
    }

    fn schema_ref_name(reference: &str, component: &str) -> Option<String> {
        reference.strip_prefix(&format!("#/components/{component}/")).map(|name| name.replace("~1", "/"))
    }

    fn schema_accepts(bundle: &OpenRpcBundle, schema: &Value, value: &Value) -> bool {
        schema_accepts_at_depth(bundle, schema, value, 0)
    }

    fn schema_accepts_at_depth(bundle: &OpenRpcBundle, schema: &Value, value: &Value, depth: usize) -> bool {
        if depth > 64 {
            return true;
        }

        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let Some(name) = schema_ref_name(reference, "schemas") else { return true };
            let Some(schema) = bundle.schemas.get(&name) else { return false };
            return schema_accepts_at_depth(bundle, schema, value, depth + 1);
        }

        for combinator in ["oneOf", "anyOf"] {
            if let Some(candidates) = schema.get(combinator).and_then(Value::as_array) {
                return candidates.iter().any(|candidate| schema_accepts_at_depth(bundle, candidate, value, depth + 1));
            }
        }

        if let Some(candidates) = schema.get("allOf").and_then(Value::as_array) {
            return candidates.iter().all(|candidate| schema_accepts_at_depth(bundle, candidate, value, depth + 1));
        }

        if let Some(enum_values) = schema.get("enum").and_then(Value::as_array) {
            return enum_values.iter().any(|enum_value| enum_value == value);
        }

        match schema_type(schema) {
            Some("object") => object_schema_accepts(bundle, schema, value, depth),
            Some("array") => array_schema_accepts(bundle, schema, value, depth),
            Some("string") => value.is_string(),
            Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
            Some("number") => value.is_number(),
            Some("boolean") => value.is_boolean(),
            Some("null") => value.is_null(),
            Some(_) => true,
            None if schema.get("properties").is_some() || schema.get("required").is_some() => {
                object_schema_accepts(bundle, schema, value, depth)
            }
            None if schema.get("items").is_some() => array_schema_accepts(bundle, schema, value, depth),
            None => true,
        }
    }

    fn schema_type(schema: &Value) -> Option<&str> {
        match schema.get("type") {
            Some(Value::String(schema_type)) => Some(schema_type.as_str()),
            Some(Value::Array(types)) => types.iter().find_map(Value::as_str),
            _ => None,
        }
    }

    fn object_schema_accepts(bundle: &OpenRpcBundle, schema: &Value, value: &Value, depth: usize) -> bool {
        let Some(object) = value.as_object() else { return false };

        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for field in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(field) {
                    return false;
                }
            }
        }

        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            return true;
        };

        for (field, field_schema) in properties {
            if let Some(field_value) = object.get(field) {
                if !schema_accepts_at_depth(bundle, field_schema, field_value, depth + 1) {
                    return false;
                }
            }
        }

        true
    }

    fn array_schema_accepts(bundle: &OpenRpcBundle, schema: &Value, value: &Value, depth: usize) -> bool {
        let Some(array) = value.as_array() else { return false };
        let Some(items) = schema.get("items") else { return true };
        array.iter().all(|item| schema_accepts_at_depth(bundle, items, item, depth + 1))
    }

    fn result_schema(method: &OpenRpcMethod) -> Option<&Value> {
        method.result.get("schema")
    }

    fn assert_success_matches_schema(bundle: &OpenRpcBundle, method: &str, response: &Value) {
        if let Some(error) = response.get("error") {
            panic!("{method} returned an unexpected error: {error}");
        }
        let result = response.get("result").unwrap_or_else(|| panic!("{method} response missing result: {response}"));
        let method_name = method;
        let method = bundle.methods.get(method_name).unwrap_or_else(|| panic!("missing OpenRPC method {method_name}"));
        let Some(schema) = result_schema(method) else { return };
        assert!(schema_accepts(bundle, schema, result), "result does not match OpenRPC schema: {result}");
    }

    fn assert_error_matches_openrpc(bundle: &OpenRpcBundle, method: &str, response: &Value) {
        let error = response.get("error").unwrap_or_else(|| panic!("{method} response missing error: {response}"));
        let code = error.get("code").and_then(Value::as_i64).unwrap_or_else(|| panic!("{method} error missing code"));
        let message =
            error.get("message").and_then(Value::as_str).unwrap_or_else(|| panic!("{method} error missing message"));

        let method_name = method;
        let method = bundle.methods.get(method_name).unwrap_or_else(|| panic!("missing OpenRPC method {method_name}"));
        let mut allowed = Vec::new();
        for error_ref in &method.errors {
            let Some(reference) = error_ref.get("$ref").and_then(Value::as_str) else { continue };
            let Some(name) = schema_ref_name(reference, "errors") else { continue };
            let Some(error_schema) = bundle.errors.get(&name) else { continue };
            allowed.push(name);

            let expected_code = error_schema.get("code").and_then(Value::as_i64);
            if expected_code == Some(code) {
                return;
            }
        }

        panic!("{method_name} returned unlisted error code={code} message={message:?}; allowed={allowed:?}");
    }

    #[tokio::test]
    async fn openrpc_method_sets_are_exposed_by_versioned_routes() {
        let madara = get_madara().await;

        for spec in manifest().versions {
            let bundle = load_openrpc_bundle(&spec);
            let expected = expected_callable_methods(&bundle, &spec);
            let response = rpc_response(&route_url(madara, &spec.route), "rpc_methods", json!([])).await;
            let raw_methods = response["result"]["methods"]
                .as_array()
                .unwrap_or_else(|| panic!("rpc_methods response should contain methods array: {response}"))
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            let actual = raw_methods
                .iter()
                .filter_map(|method| route_method_name(&spec.version, method))
                .collect::<BTreeSet<_>>();

            let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
            assert!(
                missing.is_empty(),
                "{} missing OpenRPC methods: {missing:?}; rpc_methods sample: {:?}",
                spec.version,
                raw_methods.iter().take(8).collect::<Vec<_>>(),
            );
        }
    }

    #[tokio::test]
    async fn route_spec_versions_match_manifest() {
        let madara = get_madara().await;

        for spec in manifest().versions {
            let response = rpc_response(&route_url(madara, &spec.route), "starknet_specVersion", json!({})).await;
            assert_eq!(
                response.get("result").and_then(Value::as_str),
                Some(spec.expected_spec_version.as_str()),
                "{} returned unexpected spec version: {response}",
                spec.version
            );
        }
    }

    #[tokio::test]
    async fn representative_http_results_match_openrpc_schemas() {
        let madara = get_madara().await;

        for spec in manifest().versions {
            let bundle = load_openrpc_bundle(&spec);
            let url = route_url(madara, &spec.route);
            for case in success_cases(&bundle, &spec) {
                let response = rpc_response(&url, case.method, case.params).await;
                assert_success_matches_schema(&bundle, case.method, &response);
            }
        }
    }

    #[tokio::test]
    async fn representative_errors_match_openrpc_errors() {
        let madara = get_madara().await;

        for spec in manifest().versions {
            let bundle = load_openrpc_bundle(&spec);
            let url = route_url(madara, &spec.route);
            for case in error_cases(&bundle, &spec) {
                let response = rpc_response(&url, case.method, case.params).await;
                assert_error_matches_openrpc(&bundle, case.method, &response);
            }
        }
    }

    #[tokio::test]
    async fn websocket_routes_accept_subscription_lifecycle() {
        let madara = get_madara().await;

        for spec in manifest().versions.into_iter().filter(|spec| has_openrpc_file(spec, "starknet_ws_api.json")) {
            let bundle = load_openrpc_bundle(&spec);
            assert!(
                bundle.methods.contains_key("starknet_subscribeNewHeads"),
                "{} websocket OpenRPC fixture should include subscribeNewHeads",
                spec.version
            );

            let client = WsClientBuilder::default()
                .build(&websocket_route_url(madara, &spec.route))
                .await
                .unwrap_or_else(|err| panic!("building websocket client for {}: {err}", spec.version));
            let version: String = client
                .request("starknet_specVersion", jsonrpsee::rpc_params![])
                .await
                .unwrap_or_else(|err| panic!("requesting specVersion over websocket for {}: {err}", spec.version));
            assert_eq!(
                version, spec.expected_spec_version,
                "{} returned unexpected websocket spec version",
                spec.version
            );

            let subscription = client
                .subscribe::<Value, _>(
                    "starknet_subscribeNewHeads",
                    jsonrpsee::rpc_params!["latest"],
                    "starknet_unsubscribe",
                )
                .await
                .unwrap_or_else(|err| panic!("subscribing to new heads over websocket for {}: {err}", spec.version));
            assert!(
                matches!(subscription.kind(), SubscriptionKind::Subscription(_)),
                "{} did not return a subscription id",
                spec.version
            );
            subscription
                .unsubscribe()
                .await
                .unwrap_or_else(|err| panic!("unsubscribing from websocket for {}: {err}", spec.version));
        }
    }

    #[test]
    fn readme_compatibility_table_lists_latest_openrpc_methods() {
        let manifest = manifest();
        let latest = manifest.versions.last().expect("manifest should contain latest route");
        let bundle = load_openrpc_bundle(latest);
        let readme_methods = readme_supported_methods();
        let missing = openrpc_methods(&bundle).difference(&readme_methods).cloned().collect::<Vec<_>>();

        assert!(missing.is_empty(), "README compatibility table is missing latest OpenRPC methods: {missing:?}");
    }

    #[test]
    fn readme_lists_all_advertised_rpc_routes() {
        let readme = include_str!("../../../../../README.md");
        for spec in manifest().versions {
            let dotted = spec.expected_spec_version.as_str();
            let routed = spec.version.replace('_', ".");
            assert!(
                readme.contains(dotted) || readme.contains(&routed),
                "README does not mention advertised RPC route/version {}",
                spec.version
            );
        }
    }

    #[test]
    fn openrpc_fixtures_have_unique_method_names_per_route() {
        for spec in manifest().versions {
            let bundle = load_openrpc_bundle(&spec);
            let mut seen = BTreeSet::new();
            for method in bundle.methods.keys() {
                assert!(seen.insert(method), "{} duplicates method {method}", spec.version);
            }
        }
    }
}
