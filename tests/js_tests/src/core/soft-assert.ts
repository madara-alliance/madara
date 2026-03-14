export class SoftAssertCollector {
  private readonly todos = new Set<string>();

  range(
    key: string,
    value: bigint,
    boundaries: { min?: bigint; max?: bigint },
  ): void {
    if (boundaries.min !== undefined && value < boundaries.min) {
      throw new Error(
        `${key} is below minimum bound. got=${value} min=${boundaries.min}`,
      );
    }

    if (boundaries.max !== undefined && value > boundaries.max) {
      throw new Error(
        `${key} is above maximum bound. got=${value} max=${boundaries.max}`,
      );
    }
  }

  todo(id: string, note: string): void {
    this.todos.add(`${id}: ${note}`);
  }

  flush(prefix = "SOFT_ASSERT_TODO"): void {
    if (this.todos.size === 0) {
      return;
    }

    for (const todo of this.todos) {
      // eslint-disable-next-line no-console
      console.warn(`${prefix} ${todo}`);
    }
  }
}
