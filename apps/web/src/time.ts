// Snowflake → time (ADR-0005: 42 bits of ms since 2026-01-01T00:00:00Z).
export const EPOCH_MS = 1_767_225_600_000n;

export function snowflakeTime(id: bigint): number {
  return Number((id >> 22n) + EPOCH_MS);
}
