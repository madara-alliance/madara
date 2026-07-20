use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Scarb target directory containing *.starknet_artifacts.json and contract_class.json files.
    #[arg(long)]
    scarb_target_dir: PathBuf,
    /// Root of paradex_contracts (for parsing Cairo storage layout).
    #[arg(long)]
    cairo_root: PathBuf,
    /// Output directory for generated Rust modules.
    #[arg(long)]
    out_dir: PathBuf,
    /// Comma-separated list of logical contracts: paraclear,oracle,assets_manager.
    #[arg(long)]
    contracts: String,
}

#[derive(Debug, Deserialize)]
struct StarknetArtifacts {
    contracts: Vec<ArtifactContract>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ArtifactContract {
    contract_name: String,
    module_path: String,
    artifacts: ArtifactFiles,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ArtifactFiles {
    sierra: String,
    casm: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContractClass {
    abi: Vec<AbiItem>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum AbiItem {
    #[serde(rename = "struct")]
    Struct { name: String, members: Vec<AbiMember> },
    #[serde(rename = "enum")]
    Enum { name: String, variants: Vec<AbiVariant> },
    #[serde(rename = "event")]
    Event { name: String },
    #[serde(rename = "interface")]
    Interface { name: String },
    #[serde(rename = "impl")]
    Impl { name: String, interface_name: String },
    #[serde(rename = "constructor")]
    Constructor { name: String },
    #[serde(rename = "function")]
    Function { name: String },
}

#[derive(Debug, Deserialize, Clone)]
struct AbiMember {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[derive(Debug, Deserialize, Clone)]
struct AbiVariant {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let contracts = parse_contracts(&args.contracts);
    if contracts.is_empty() {
        bail!("no contracts specified");
    }

    fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("failed to create out dir: {}", args.out_dir.display()))?;

    for contract in &contracts {
        let (artifact_file, contract_name) = resolve_contract_artifact(&args.scarb_target_dir, contract)?;
        let contract_class_path = args.scarb_target_dir.join(artifact_file);

        let data = fs::read_to_string(&contract_class_path)
            .with_context(|| format!("failed to read {}", contract_class_path.display()))?;
        let contract_class: ContractClass = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse ABI from {}", contract_class_path.display()))?;

        let types = extract_types(&contract_class.abi);
        let module_name = format!("{}_types.rs", contract);
        let out_path = args.out_dir.join(module_name);
        fs::write(&out_path, render_types(&contract_name, &types))
            .with_context(|| format!("failed to write {}", out_path.display()))?;

        let layout_name = format!("{}_layout.rs", contract);
        let layout_path = args.out_dir.join(layout_name);
        let layout_src = resolve_layout_source(&args.cairo_root, contract)?;
        let fields = parse_storage_fields(&layout_src)?;
        fs::write(&layout_path, render_layout(&contract_name, &fields))
            .with_context(|| format!("failed to write {}", layout_path.display()))?;
    }

    write_component_layouts(&args.cairo_root, &args.out_dir)?;

    write_mod_rs(&args.out_dir, &contracts)?;

    Ok(())
}

fn write_component_layouts(cairo_root: &Path, out_dir: &Path) -> Result<()> {
    let components = [
        ("account_component", "paraclear/src/account/account.cairo"),
        ("token_component", "paraclear/src/token/token.cairo"),
        ("perpetual_asset_component", "paraclear/src/perpetual/perpetual_asset.cairo"),
        ("perpetual_future_component", "paraclear/src/perpetual/future.cairo"),
        ("perpetual_option_component", "paraclear/src/perpetual/option.cairo"),
        ("assets_spot_component", "assets_manager/src/spot.cairo"),
        ("assets_future_component", "assets_manager/src/future.cairo"),
        ("assets_option_component", "assets_manager/src/option.cairo"),
        ("assets_token_component", "assets_manager/src/token.cairo"),
    ];

    for (name, rel) in components {
        let path = cairo_root.join(rel);
        if !path.exists() {
            continue;
        }
        let fields = parse_storage_fields(&path)?;
        let out_path = out_dir.join(format!("{name}_layout.rs"));
        fs::write(&out_path, render_layout(name, &fields))
            .with_context(|| format!("failed to write {}", out_path.display()))?;
    }

    Ok(())
}

fn parse_contracts(raw: &str) -> Vec<String> {
    raw.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
}

fn resolve_contract_artifact(scarb_target_dir: &Path, contract: &str) -> Result<(String, String)> {
    let (artifact_file, contract_name) = match contract {
        "paraclear" => ("paradex_paraclear.starknet_artifacts.json", "Paraclear"),
        "oracle" => ("paradex_oracle.starknet_artifacts.json", "ParaclearOracle"),
        "assets_manager" => ("paradex_assets_manager.starknet_artifacts.json", "AssetsManager"),
        _ => bail!("unknown contract key: {contract}"),
    };

    let artifacts_path = scarb_target_dir.join(artifact_file);
    let data =
        fs::read_to_string(&artifacts_path).with_context(|| format!("failed to read {}", artifacts_path.display()))?;
    let artifacts: StarknetArtifacts =
        serde_json::from_str(&data).with_context(|| format!("failed to parse {}", artifacts_path.display()))?;

    for entry in artifacts.contracts {
        if entry.contract_name == contract_name {
            return Ok((entry.artifacts.sierra, contract_name.to_string()));
        }
    }

    bail!("contract {contract_name} not found in {}", artifacts_path.display())
}

#[derive(Debug, Default)]
struct AbiTypes {
    structs: Vec<AbiStruct>,
    enums: Vec<AbiEnum>,
}

#[derive(Debug)]
struct AbiStruct {
    name: String,
    members: Vec<AbiMember>,
}

#[derive(Debug)]
struct AbiEnum {
    name: String,
    variants: Vec<AbiVariant>,
}

fn extract_types(abi: &[AbiItem]) -> AbiTypes {
    let mut types = AbiTypes::default();

    for item in abi {
        match item {
            AbiItem::Struct { name, members } => {
                if should_emit_type(name) {
                    types.structs.push(AbiStruct { name: name.clone(), members: members.clone() });
                }
            }
            AbiItem::Enum { name, variants } => {
                if should_emit_type(name) {
                    types.enums.push(AbiEnum { name: name.clone(), variants: variants.clone() });
                }
            }
            _ => {}
        }
    }

    types
}

fn render_types(contract_name: &str, types: &AbiTypes) -> String {
    let mut out = String::new();
    out.push_str("// @generated by paradex_codegen\n");
    out.push_str(&format!("// Contract: {}\n\n", contract_name));
    out.push_str("use crate::types::ContractAddress;\n");
    out.push_str("use starknet_types_core::felt::Felt;\n\n");

    let mut name_map: HashMap<String, String> = HashMap::new();
    name_map.insert("core::integer::u256".to_string(), "U256".to_string());

    for s in &types.structs {
        name_map.insert(s.name.clone(), rust_type_name(&s.name));
    }
    for e in &types.enums {
        name_map.insert(e.name.clone(), rust_type_name(&e.name));
    }

    let needs_u256 = types
        .structs
        .iter()
        .flat_map(|s| s.members.iter().map(|m| m.ty.as_str()))
        .any(|ty| ty.contains("core::integer::u256"));
    if needs_u256 {
        out.push_str("#[derive(Clone, Debug, PartialEq)]\n");
        out.push_str("pub struct U256 {\n    pub low: u128,\n    pub high: u128,\n}\n\n");
    }

    for s in &types.structs {
        out.push_str("#[derive(Clone, Debug, PartialEq)]\n");
        out.push_str(&format!("pub struct {} {{\n", rust_type_name(&s.name)));
        for m in &s.members {
            let ty = map_type(&m.ty, &name_map);
            out.push_str(&format!("    pub {}: {},\n", rust_field_name(&m.name), ty));
        }
        out.push_str("}\n\n");
    }

    for e in &types.enums {
        out.push_str("#[derive(Clone, Debug, PartialEq)]\n");
        out.push_str(&format!("pub enum {} {{\n", rust_type_name(&e.name)));
        for v in &e.variants {
            if v.ty == "()" {
                out.push_str(&format!("    {},\n", rust_variant_name(&v.name)));
            } else {
                let ty = map_type(&v.ty, &name_map);
                out.push_str(&format!("    {}({}),\n", rust_variant_name(&v.name), ty));
            }
        }
        out.push_str("}\n\n");
    }

    out
}

#[allow(dead_code)]
fn render_layout_stub(contract_name: &str) -> String {
    format!(
        "// @generated by paradex_codegen\n// Contract: {}\n\n// TODO: Fill storage layout constants using compiler-derived addresses.\n",
        contract_name
    )
}

fn resolve_layout_source(cairo_root: &Path, contract: &str) -> Result<PathBuf> {
    let rel = match contract {
        "paraclear" => "paraclear/src/paraclear/paraclear.cairo",
        "oracle" => "oracle/src/oracle.cairo",
        "assets_manager" => "assets_manager/src/assets_manager.cairo",
        _ => bail!("unknown contract key: {contract}"),
    };
    Ok(cairo_root.join(rel))
}

#[derive(Debug)]
struct StorageField {
    name: String,
    ty: String,
}

fn parse_storage_fields(path: &Path) -> Result<Vec<StorageField>> {
    let data = fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;

    let start = data.find("#[storage]").ok_or_else(|| anyhow::anyhow!("no #[storage] in {}", path.display()))?;
    let rest = &data[start..];
    let struct_pos =
        rest.find("struct Storage").ok_or_else(|| anyhow::anyhow!("no Storage struct in {}", path.display()))?;
    let idx = start + struct_pos;

    let open_idx = data[idx..].find('{').ok_or_else(|| anyhow::anyhow!("no '{{' in Storage struct"))? + idx;
    let mut depth = 0i32;
    let mut end_idx = None;
    for (i, ch) in data[open_idx..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end_idx = Some(open_idx + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let end_idx = end_idx.ok_or_else(|| anyhow::anyhow!("no closing '}}' in Storage struct"))?;
    let body = &data[open_idx + 1..end_idx];

    let statements = split_statements(body);
    let mut fields = Vec::new();
    for stmt in statements {
        let filtered = stmt.lines().filter(|l| !l.trim_start().starts_with('#')).collect::<Vec<_>>().join(" ");
        let line = strip_comments(filtered.trim());
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, ty)) = parse_field(line) {
            fields.push(StorageField { name, ty });
        }
    }

    Ok(fields)
}

fn split_statements(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut depth_angle = 0i32;
    let mut depth_paren = 0i32;
    let mut chars = body.chars().peekable();
    let mut in_comment = false;

    while let Some(ch) = chars.next() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                buf.push(ch);
            }
            continue;
        }

        if ch == '/' {
            if let Some('/') = chars.peek().copied() {
                chars.next();
                in_comment = true;
                continue;
            }
        }

        match ch {
            '<' => depth_angle += 1,
            '>' => depth_angle -= 1,
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            ',' if depth_angle == 0 && depth_paren == 0 => {
                out.push(buf.clone());
                buf.clear();
                continue;
            }
            _ => {}
        }
        buf.push(ch);
    }
    if !buf.trim().is_empty() {
        out.push(buf);
    }
    out
}

fn strip_comments(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

fn parse_field(line: &str) -> Option<(String, String)> {
    let mut parts = line.splitn(2, ':');
    let mut name = parts.next()?.trim().to_string();
    let ty = parts.next()?.trim();
    if name.starts_with("pub ") {
        name = name.trim_start_matches("pub ").trim().to_string();
    }
    if name.is_empty() || ty.is_empty() {
        return None;
    }
    Some((name, ty.to_string()))
}

fn render_layout(contract_name: &str, fields: &[StorageField]) -> String {
    let mut out = String::new();
    out.push_str("// @generated by paradex_codegen\n");
    out.push_str(&format!("// Contract: {}\n\n", contract_name));
    out.push_str("use once_cell::sync::Lazy;\n");
    out.push_str("use starknet_types_core::felt::Felt;\n");
    out.push_str("use crate::storage::{storage_key_for_map_poseidon, storage_key_for_map2_poseidon, storage_key_for_variable};\n");
    out.push_str("use crate::types::StorageKey;\n\n");

    for field in fields {
        let const_name = field.name.to_uppercase();
        out.push_str(&format!(
            "pub static {const_name}_BASE: Lazy<StorageKey> = Lazy::new(|| storage_key_for_variable(\"{name}\"));\n",
            const_name = const_name,
            name = field.name,
        ));

        if let Some(map) = parse_map_type(&field.ty) {
            match map.key_count {
                1 => {
                    out.push_str(&format!(
                        "pub fn {name}_key(k: Felt) -> StorageKey {{ storage_key_for_map_poseidon(\"{name}\", k) }}\n",
                        name = field.name,
                    ));
                }
                2 => {
                    out.push_str(&format!(
                        "pub fn {name}_key2(k1: Felt, k2: Felt) -> StorageKey {{ storage_key_for_map2_poseidon(\"{name}\", k1, k2) }}\n",
                        name = field.name,
                    ));
                }
                _ => {}
            }
        }

        out.push('\n');
    }

    out
}

struct MapType {
    key_count: usize,
}

fn parse_map_type(ty: &str) -> Option<MapType> {
    let ty = ty.replace(' ', "");
    let inner = ty.strip_prefix("Map<")?.strip_suffix('>')?;
    if inner.starts_with('(') {
        let end = inner.find(')')?;
        let tuple = &inner[1..end];
        let count = tuple.split(',').filter(|s| !s.is_empty()).count();
        return Some(MapType { key_count: count });
    }
    Some(MapType { key_count: 1 })
}

fn write_mod_rs(out_dir: &Path, contracts: &[String]) -> Result<()> {
    let mut out = String::new();
    out.push_str("// @generated by paradex_codegen\n\n");
    for contract in contracts {
        out.push_str(&format!("pub mod {}_types;\n", contract));
        out.push_str(&format!("pub mod {}_layout;\n", contract));
    }
    out.push_str("pub mod account_component_layout;\n");
    out.push_str("pub mod token_component_layout;\n");
    out.push_str("pub mod perpetual_asset_component_layout;\n");
    out.push_str("pub mod perpetual_future_component_layout;\n");
    out.push_str("pub mod perpetual_option_component_layout;\n");
    out.push_str("pub mod assets_spot_component_layout;\n");
    out.push_str("pub mod assets_future_component_layout;\n");
    out.push_str("pub mod assets_option_component_layout;\n");
    out.push_str("pub mod assets_token_component_layout;\n");

    let path = out_dir.join("mod.rs");
    fs::write(&path, out).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn rust_type_name(name: &str) -> String {
    let base = name.split("::").last().unwrap_or(name);
    base.split('<').next().unwrap_or(base).to_string()
}

fn rust_field_name(name: &str) -> String {
    if name == "type" {
        "type_".to_string()
    } else {
        name.to_string()
    }
}

fn rust_variant_name(name: &str) -> String {
    name.to_string()
}

fn map_type(ty: &str, name_map: &HashMap<String, String>) -> String {
    let ty = ty.strip_prefix('@').unwrap_or(ty);

    if let Some(mapped) = name_map.get(ty) {
        return mapped.clone();
    }

    match ty {
        "core::felt252" => "Felt".to_string(),
        "core::bool" => "bool".to_string(),
        "core::integer::u8" => "u8".to_string(),
        "core::integer::u16" => "u16".to_string(),
        "core::integer::u32" => "u32".to_string(),
        "core::integer::u64" => "u64".to_string(),
        "core::integer::u128" => "u128".to_string(),
        "core::integer::i128" => "i128".to_string(),
        "core::integer::u256" => "U256".to_string(),
        "starknet::contract_address::ContractAddress" => "ContractAddress".to_string(),
        "core::internal::bounded_int::BoundedInt::<0, 30>" => "u32".to_string(),
        "core::bytes_31::bytes31" => "[u8; 31]".to_string(),
        _ => {
            if let Some(inner) = ty.strip_prefix("core::array::Array::<") {
                return format!("Vec<{}>", map_type(trim_generic(inner), name_map));
            }
            if let Some(inner) = ty.strip_prefix("core::span::Span::<") {
                return format!("Vec<{}>", map_type(trim_generic(inner), name_map));
            }
            if let Some(inner) = ty.strip_prefix("core::option::Option::<") {
                return format!("Option<{}>", map_type(trim_generic(inner), name_map));
            }
            if let Some(inner) = ty.strip_prefix("core::result::Result::<") {
                let (a, b) = split_two(trim_generic(inner));
                return format!("Result<{}, {}>", map_type(a, name_map), map_type(b, name_map));
            }

            // Fallback: use last segment as a best-effort type name
            rust_type_name(ty)
        }
    }
}

fn should_emit_type(name: &str) -> bool {
    if name.contains('<') || name.contains('>') {
        return false;
    }
    if name == "core::integer::u256" || name == "core::bool" {
        return false;
    }
    true
}

fn trim_generic(raw: &str) -> &str {
    raw.strip_suffix('>').unwrap_or(raw)
}

fn split_two(raw: &str) -> (&str, &str) {
    let mut depth = 0usize;
    for (idx, ch) in raw.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                let a = raw[..idx].trim();
                let b = raw[idx + 1..].trim();
                return (a, b);
            }
            _ => {}
        }
    }
    (raw, "")
}
