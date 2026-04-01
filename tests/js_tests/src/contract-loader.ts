import * as fs from "fs";
import * as path from "path";
import { CompiledSierra, CompiledSierraCasm } from "starknet";

const BUILD_ARTIFACTS_DIR = path.resolve(
  __dirname,
  "../../../build-artifacts/js_tests",
);
const CAIRO_ARTIFACTS_DIR = path.resolve(
  __dirname,
  "../../../build-artifacts/cairo_artifacts",
);

export function loadContractSierra(name: string): CompiledSierra {
  const filePath = path.join(
    BUILD_ARTIFACTS_DIR,
    `${name}.contract_class.json`,
  );
  return JSON.parse(fs.readFileSync(filePath, "utf-8"));
}

export function loadContractCasm(name: string): CompiledSierraCasm {
  const filePath = path.join(
    BUILD_ARTIFACTS_DIR,
    `${name}.compiled_contract_class.json`,
  );
  return JSON.parse(fs.readFileSync(filePath, "utf-8"));
}

export function loadCairoArtifactSierra(name: string): CompiledSierra {
  const filePath = path.join(
    CAIRO_ARTIFACTS_DIR,
    `${name}.contract_class.json`,
  );
  return JSON.parse(fs.readFileSync(filePath, "utf-8"));
}
