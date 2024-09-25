import {
    CompiledSierra,
    CompiledSierraCasm,
    json,
    LegacyCompiledContract
  } from "starknet";
  const fs = require("node:fs");
  const path = require("node:path");
  export const readContract = (name: string): LegacyCompiledContract =>
    json.parse(
      fs
        .readFileSync(path.resolve(__dirname, `./mocks/${name}.json`))
        .toString("ascii"),
    );
  
  export const readContractSierraCasm = (name: string): CompiledSierraCasm =>
    json.parse(
      fs
        .readFileSync(path.resolve(__dirname, `../cairo/target/dev/${name}.compiled_contract_class.json`))
        .toString("ascii"),
    );
  
  export const readContractSierra = (name: string): CompiledSierra =>
    json.parse(
      fs
        .readFileSync(path.resolve(__dirname, `../cairo/target/dev/${name}.contract_class.json`))
        .toString("ascii"),
    );