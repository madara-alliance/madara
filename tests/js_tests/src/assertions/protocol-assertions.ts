import type {
  JsonRpcEnvelopeError,
  JsonRpcEnvelopeSuccess,
} from "../config/types";

export function assertEnvelope(
  envelope: JsonRpcEnvelopeSuccess<unknown> | JsonRpcEnvelopeError,
): void {
  expect(envelope.jsonrpc).toBe("2.0");
  expect("id" in envelope).toBe(true);
}

export function assertSuccessEnvelope(
  envelope: JsonRpcEnvelopeSuccess<unknown> | JsonRpcEnvelopeError,
): asserts envelope is JsonRpcEnvelopeSuccess<unknown> {
  assertEnvelope(envelope);
  expect("result" in envelope).toBe(true);
  expect("error" in envelope).toBe(false);
}

export function assertErrorEnvelope(
  envelope: JsonRpcEnvelopeSuccess<unknown> | JsonRpcEnvelopeError,
): asserts envelope is JsonRpcEnvelopeError {
  assertEnvelope(envelope);
  expect("error" in envelope).toBe(true);
  if ("error" in envelope) {
    expect(typeof envelope.error.code).toBe("number");
    expect(typeof envelope.error.message).toBe("string");
  }
}
