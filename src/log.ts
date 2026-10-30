export function logError(context: string, error: unknown): void {
  const detail = error instanceof Error ? error.stack || error.message : String(error);
  console.error(`[Buho] ${context}: ${detail}`);
}
