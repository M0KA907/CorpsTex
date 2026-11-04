export type ChartData = { type: 'bar' | 'line'; title: string; data: { label: string; value: number }[] };
const record = (value: unknown): value is Record<string, unknown> => Boolean(value && typeof value === 'object' && !Array.isArray(value));
function keys(value: Record<string, unknown>, allowed: string[]) {
  if (Object.keys(value).some(key => !allowed.includes(key))) throw new Error('Unsupported field in preview data.');
}
function label(value: unknown, max = 160): string {
  if (typeof value !== 'string' || !value.trim() || value.length > max) throw new Error('A nonempty, bounded text label is required.');
  return value;
}
function number(value: unknown, limit: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || Math.abs(value) > limit) throw new Error('Preview coordinates must be finite and within range.');
  return value;
}
function parse(source: string): Record<string, unknown> {
  if (source.length > 131072) throw new Error('Preview exceeds the 128 KiB character limit.');
  const value: unknown = JSON.parse(source);
  if (!record(value)) throw new Error('Preview must be a JSON object.');
  return value;
}
export function parseChart(source: string): ChartData {
  const value = parse(source);
  keys(value, ['type', 'title', 'data']);
  if (value.type !== 'bar' && value.type !== 'line') throw new Error('Supported chart types: bar, line.');
  if (!Array.isArray(value.data) || !value.data.length || value.data.length > 200) throw new Error('Charts require 1–200 data points.');
  return { type: value.type, title: label(value.title), data: value.data.map(row => {
    if (!record(row)) throw new Error('Expected a data point.');
    keys(row, ['label', 'value']);
    return { label: label(row.label), value: number(row.value, 1e12) };
  }) };
}
