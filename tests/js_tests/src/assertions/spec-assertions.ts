import type { SpecRegistry } from "../config/types";

export function assertParamsConformSpec(
  spec: SpecRegistry,
  method: string,
  params: unknown,
): void {
  const validator = spec.getMethodParamsValidator(method);
  expect(validator(params)).toBe(true);
}

export function assertResultConformsSpec(
  spec: SpecRegistry,
  method: string,
  result: unknown,
): void {
  const validator = spec.getMethodResultValidator(method);
  expect(validator(result)).toBe(true);
}
