/** Presentation helpers. Pure functions, so they are trivially testable. */

const KIB = 1024;

export function formatBytes(bytes: number): string {
  if (bytes < KIB) return `${bytes} B`;
  const kib = bytes / KIB;
  if (kib < KIB) return `${kib.toFixed(kib < 10 ? 1 : 0)} KB`;
  return `${(kib / KIB).toFixed(2)} MB`;
}

/** Signed percentage change from `before` to `after`, e.g. "−64%". */
export function formatDelta(before: number, after: number): string {
  if (before === 0) return "—";
  const change = Math.round(((after - before) / before) * 100);
  if (change === 0) return "no change";
  const sign = change < 0 ? "−" : "+";
  return `${sign}${Math.abs(change)}%`;
}

export function baseName(fileName: string): string {
  const dot = fileName.lastIndexOf(".");
  return dot > 0 ? fileName.slice(0, dot) : fileName;
}
