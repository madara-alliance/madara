import { bindSpec, buildRpcTarget, resolveMatrixFromEnv } from "../config/matrix";
import { normalizeSemver, toRpcPathVersion } from "../core/semver";
import { createSpecRegistry } from "../spec/spec-registry";
import { SPEC_LOCK_MANIFEST } from "../spec/spec-sync-manifest";
import { ensureSpecCached } from "../spec/spec-cache";

describe("Unit Harness", () => {
  test("normalizes semver and rpc path version correctly", () => {
    expect(normalizeSemver("v0_10_0")).toBe("0.10.0");
    expect(toRpcPathVersion("0.10.1")).toBe("v0_10_1");
  });

  test("spec binding supports exact and fallback minors", () => {
    expect(bindSpec("0.10.0").specTag).toBe("v0.10.0");
    expect(bindSpec("0.10.1").specTag).toBe("v0.10.0");
    expect(bindSpec("0.9.2").specTag).toBe("v0.9.0");
    expect(() => bindSpec("0.8.1")).toThrow();
  });

  test("matrix honors RPC_VERSION env", () => {
    process.env.RPC_VERSION = "0.9.0";
    expect(resolveMatrixFromEnv()).toEqual(["0.9.0"]);
    delete process.env.RPC_VERSION;
  });

  test("buildRpcTarget produces versioned URL", () => {
    const target = buildRpcTarget("0.9.0", "http://127.0.0.1:9944");
    expect(target.baseUrl).toBe("http://127.0.0.1:9944/rpc/v0_9_0/");
  });

  test("spec lock manifest has pinned entries", () => {
    expect(SPEC_LOCK_MANIFEST.length).toBeGreaterThanOrEqual(2);
    const v10 = SPEC_LOCK_MANIFEST.find((entry) => entry.tag === "v0.10.0");
    expect(v10).toBeDefined();
    expect(v10?.files.length).toBe(4);
  });

  test("spec registry loads validators from cached official OpenRPC docs", async () => {
    const binding = bindSpec("0.10.0");
    await ensureSpecCached(binding);
    const registry = createSpecRegistry(binding);

    expect(registry.listOfficialMethods().includes("starknet_chainId")).toBe(true);

    const paramsValidator = registry.getMethodParamsValidator("starknet_chainId");
    const resultValidator = registry.getMethodResultValidator("starknet_chainId");

    expect(paramsValidator([])).toBe(true);
    expect(resultValidator("0x534e5f4d41494e")).toBe(true);
  });

  test("spec registry resolves cross-file refs for write and trace schemas", async () => {
    const binding = bindSpec("0.10.0");
    await ensureSpecCached(binding);
    const registry = createSpecRegistry(binding);

    expect(() =>
      registry.getMethodParamsValidator("starknet_addInvokeTransaction"),
    ).not.toThrow();
    expect(() =>
      registry.getMethodParamsValidator("starknet_traceTransaction"),
    ).not.toThrow();
  });
});
