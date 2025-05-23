import { CompiledSierra, CompiledSierraCasm, json } from "starknet";
const fs = require("node:fs");
const path = require("node:path");

export const readContractCasm = (name: string): CompiledSierraCasm =>
  json.parse(
    fs
      .readFileSync(
        path.resolve(
          __dirname,
          `../../build_artifacts/js_tests/${name}.compiled_contract_class.json`,
        ),
      )
      .toString("ascii"),
  );

export const readContractSierra = (name: string): CompiledSierra =>
  json.parse(
    fs
      .readFileSync(
        path.resolve(
          __dirname,
          `../../build_artifacts/js_tests/${name}.contract_class.json`,
        ),
      )
      .toString("ascii"),
  );

export const readContractSierraInArtifacts = (name: string): CompiledSierra =>
  json.parse(
    fs
      .readFileSync(
        path.resolve(
          __dirname,
          `../../build_artifacts/cairo_artifacts/${name}.contract_class.json`,
        ),
      )
      .toString("ascii"),
  );
