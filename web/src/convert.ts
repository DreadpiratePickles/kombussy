/**
 * Thin typed wrapper over the kombussy wasm module.
 *
 * The wasm binary is the largest asset in the app, so it is loaded lazily and
 * only once: nothing is fetched until the user actually supplies a font.
 */
import init, {
  convert_font,
  detect_format,
  output_extension,
  table_report,
} from "./wasm/kombussy_wasm.js";
import wasmUrl from "./wasm/kombussy_wasm_bg.wasm?url";

export type TargetFormat = "ttf" | "otf" | "woff" | "woff2";

export interface FontTable {
  readonly tag: string;
  readonly byteLength: number;
}

export interface FontSummary {
  readonly container: string;
  readonly tables: readonly FontTable[];
}

export interface ConversionResult {
  readonly bytes: Uint8Array;
  readonly extension: string;
  readonly byteLength: number;
}

let ready: Promise<void> | undefined;

/** Instantiate the wasm module once and reuse it for every later call. */
function load(): Promise<void> {
  ready ??= init({ module_or_path: wasmUrl }).then(() => undefined);
  return ready;
}

/** Normalise the wasm boundary's thrown values into a real Error. */
function toError(cause: unknown, fallback: string): Error {
  if (cause instanceof Error) return cause;
  if (typeof cause === "string") return new Error(cause);
  return new Error(fallback);
}

export async function summarise(input: Uint8Array): Promise<FontSummary> {
  await load();
  try {
    const container = detect_format(input);
    const report = table_report(input);
    const tables = report
      .split("\n")
      .filter((line) => line.length > 0)
      .map((line) => {
        const [tag = "", size = "0"] = line.split("\t");
        return { tag, byteLength: Number.parseInt(size, 10) };
      });
    return { container, tables };
  } catch (cause) {
    throw toError(cause, "this file could not be read as a font");
  }
}

export async function convert(input: Uint8Array, target: TargetFormat): Promise<ConversionResult> {
  await load();
  try {
    const bytes = convert_font(input, target);
    return { bytes, extension: output_extension(input, target), byteLength: bytes.length };
  } catch (cause) {
    throw toError(cause, `conversion to ${target} failed`);
  }
}
