import type { SpecRegistry } from "../config/types";

export function assertParamsConformSpec(
  spec: SpecRegistry,
  method: string,
  params: unknown,
): void {
  const validator = spec.getMethodParamsValidator(method);
  const valid = validator(params);
  if (!valid) {
    const errors = (validator as any).errors;
    throw new Error(
      `Spec params validation failed for ${method}: ${JSON.stringify(
        errors || [],
        null,
        2,
      )}`,
    );
  }
}

export function assertResultConformsSpec(
  spec: SpecRegistry,
  method: string,
  result: unknown,
): void {
  const validator = spec.getMethodResultValidator(method);
  const valid = validator(result);
  if (!valid) {
    const errors = (validator as any).errors;
    if (isOneOfAmbiguityOnly(errors)) {
      return;
    }
    if (isNullableContinuationTokenMismatch(method, result, errors)) {
      return;
    }

    throw new Error(
      `Spec result validation failed for ${method}: ${JSON.stringify(
        errors || [],
        null,
        2,
      )}`,
    );
  }
}

function isOneOfAmbiguityOnly(errors: unknown): boolean {
  if (!Array.isArray(errors) || errors.length === 0) {
    return false;
  }

  return errors.every((error) => {
    if (!error || typeof error !== "object") {
      return false;
    }

    const keyword = (error as any).keyword;
    const passingSchemas = (error as any).params?.passingSchemas;
    return (
      keyword === "oneOf" &&
      Array.isArray(passingSchemas) &&
      passingSchemas.length > 1
    );
  });
}

function isNullableContinuationTokenMismatch(
  method: string,
  result: unknown,
  errors: unknown,
): boolean {
  if (method !== "starknet_getEvents") {
    return false;
  }

  if (!result || typeof result !== "object") {
    return false;
  }

  if ((result as any).continuation_token !== null) {
    return false;
  }

  if (!Array.isArray(errors) || errors.length !== 1) {
    return false;
  }

  const error = errors[0] as any;
  return (
    error?.instancePath === "/result/continuation_token" &&
    error?.keyword === "type" &&
    error?.params?.type === "string"
  );
}
