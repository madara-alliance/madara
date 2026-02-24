import fs from "node:fs";
import path from "node:path";
import { json, type CompiledSierra, type CompiledSierraCasm } from "starknet";

function readJsonFile(filePath: string) {
  return json.parse(fs.readFileSync(filePath).toString("ascii"));
}

export function readContractCasm(name: string): CompiledSierraCasm {
  return readJsonFile(
    path.resolve(
      process.cwd(),
      "../../build-artifacts/js_tests",
      `${name}.compiled_contract_class.json`,
    ),
  );
}

export function readContractSierra(name: string): CompiledSierra {
  return readJsonFile(
    path.resolve(
      process.cwd(),
      "../../build-artifacts/js_tests",
      `${name}.contract_class.json`,
    ),
  );
}

export function readContractSierraInArtifacts(name: string): CompiledSierra {
  return readJsonFile(
    path.resolve(
      process.cwd(),
      "../../build-artifacts/cairo_artifacts",
      `${name}.contract_class.json`,
    ),
  );
}
