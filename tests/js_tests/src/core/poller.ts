export interface WaitForConfig {
  timeoutMs: number;
  intervalMs: number;
}

const DEFAULT_CONFIG: WaitForConfig = {
  timeoutMs: 120_000,
  intervalMs: 500,
};

export async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

export async function waitFor<T>(
  fn: () => Promise<T>,
  predicate: (value: T) => boolean,
  cfg?: Partial<WaitForConfig>,
): Promise<T> {
  const options = { ...DEFAULT_CONFIG, ...cfg };
  const start = Date.now();

  for (;;) {
    const value = await fn();
    if (predicate(value)) {
      return value;
    }

    if (Date.now() - start > options.timeoutMs) {
      throw new Error(`waitFor timed out after ${options.timeoutMs}ms`);
    }

    await sleep(options.intervalMs);
  }
}
