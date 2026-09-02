/**
 * Application shell.
 *
 * State is deliberately a single object rather than a framework: the app has
 * one document open at a time, and every render is a pure function of it.
 */
import { convert, summarise, type FontSummary, type TargetFormat } from "./convert.js";
import { baseName, formatBytes, formatDelta } from "./format.js";

const TARGETS: readonly { readonly id: TargetFormat; readonly label: string }[] = [
  { id: "ttf", label: "TTF / OTF" },
  { id: "woff", label: "WOFF" },
  { id: "woff2", label: "WOFF2" },
  { id: "otf", label: "OTF" },
];

/** Formats a browser can load through the CSS Font Loading API for preview. */
const PREVIEWABLE: ReadonlySet<TargetFormat> = new Set<TargetFormat>(["ttf", "otf", "woff", "woff2"]);

interface State {
  file: File | undefined;
  bytes: Uint8Array | undefined;
  summary: FontSummary | undefined;
  target: TargetFormat;
  previewUrl: string | undefined;
}

const state: State = {
  file: undefined,
  bytes: undefined,
  summary: undefined,
  target: "woff2",
  previewUrl: undefined,
};

function element<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
}

const ui = {
  dropzone: element<HTMLDivElement>("dropzone"),
  fileInput: element<HTMLInputElement>("file-input"),
  status: element<HTMLParagraphElement>("status"),
  workbench: element<HTMLElement>("workbench"),
  sourceName: element("source-name"),
  sourceContainer: element("source-container"),
  sourceSize: element("source-size"),
  sourceCount: element("source-count"),
  tableList: element<HTMLUListElement>("table-list"),
  targets: element<HTMLDivElement>("targets"),
  download: element<HTMLButtonElement>("download"),
  downloadLabel: element("download-label"),
  result: element<HTMLParagraphElement>("result"),
  preview: element<HTMLDivElement>("preview"),
  sample: element<HTMLInputElement>("sample"),
};

function setStatus(message: string, tone: "info" | "error" = "info"): void {
  ui.status.textContent = message;
  ui.status.dataset["tone"] = tone;
}

function setResult(html: string, tone: "info" | "error" = "info"): void {
  ui.result.innerHTML = html;
  ui.result.dataset["tone"] = tone;
}

function renderTargets(): void {
  ui.targets.replaceChildren(
    ...TARGETS.map(({ id, label }) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "target";
      button.role = "radio";
      button.textContent = label;
      button.setAttribute("aria-checked", String(state.target === id));
      button.addEventListener("click", () => {
        state.target = id;
        renderTargets();
        setResult("");
      });
      return button;
    }),
  );
}

function renderSummary(): void {
  const { file, bytes, summary } = state;
  if (!file || !bytes || !summary) return;

  ui.workbench.hidden = false;
  ui.sourceName.textContent = file.name;
  ui.sourceContainer.textContent = summary.container;
  ui.sourceSize.textContent = formatBytes(bytes.length);
  ui.sourceCount.textContent = String(summary.tables.length);

  ui.tableList.replaceChildren(
    ...summary.tables.map((table) => {
      const row = document.createElement("li");
      const tag = document.createElement("span");
      tag.textContent = table.tag;
      const size = document.createElement("span");
      size.textContent = formatBytes(table.byteLength);
      row.append(tag, size);
      return row;
    }),
  );
  ui.download.disabled = false;
}

/** Load the converted bytes back as a real web font, which proves the output
 *  is not merely well-formed but actually usable by a browser. */
async function showPreview(bytes: Uint8Array, extension: string): Promise<void> {
  if (state.previewUrl) URL.revokeObjectURL(state.previewUrl);

  // Copy into a fresh buffer: the wasm memory backing `bytes` can be
  // reallocated by a later call, which would corrupt the Blob.
  const blob = new Blob([new Uint8Array(bytes)], { type: "font/otf" });
  state.previewUrl = URL.createObjectURL(blob);

  try {
    const face = new FontFace("KombussyPreview", `url(${state.previewUrl})`);
    await face.load();
    document.fonts.add(face);
    ui.preview.style.setProperty("--preview-font", "KombussyPreview");
    ui.preview.dataset["empty"] = "false";
  } catch {
    ui.preview.style.removeProperty("--preview-font");
    ui.preview.dataset["empty"] = "true";
    setResult(
      `Converted, but this browser declined to render <strong>.${extension}</strong> as a live font.`,
    );
  }
}

async function handleFile(file: File): Promise<void> {
  ui.dropzone.classList.add("is-busy");
  setStatus(`Reading ${file.name}…`);
  setResult("");
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    const summary = await summarise(bytes);
    state.file = file;
    state.bytes = bytes;
    state.summary = summary;
    renderSummary();
    setStatus(`${file.name} — ${summary.container}, ${summary.tables.length} tables`);
  } catch (error) {
    state.file = undefined;
    state.bytes = undefined;
    state.summary = undefined;
    ui.workbench.hidden = true;
    setStatus(error instanceof Error ? error.message : "could not read that file", "error");
  } finally {
    ui.dropzone.classList.remove("is-busy");
  }
}

async function runConversion(): Promise<void> {
  const { bytes, file, target } = state;
  if (!bytes || !file) return;

  ui.download.disabled = true;
  ui.downloadLabel.textContent = "Converting…";
  try {
    const started = performance.now();
    const { bytes: output, extension, byteLength } = await convert(bytes, target);
    const elapsed = Math.max(1, Math.round(performance.now() - started));

    const name = `${baseName(file.name)}.${extension}`;
    const blob = new Blob([new Uint8Array(output)], { type: "application/octet-stream" });
    const href = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = href;
    link.download = name;
    link.click();
    // Revoke on the next task so the download has taken its reference.
    setTimeout(() => URL.revokeObjectURL(href), 0);

    setResult(
      `<strong>${name}</strong> · ${formatBytes(byteLength)} · ` +
        `${formatDelta(bytes.length, byteLength)} vs source · ${elapsed} ms`,
    );
    if (PREVIEWABLE.has(target)) await showPreview(output, extension);
  } catch (error) {
    setResult(error instanceof Error ? error.message : "conversion failed", "error");
  } finally {
    ui.download.disabled = false;
    ui.downloadLabel.textContent = "Convert";
  }
}

function wireDropzone(): void {
  const open = (): void => ui.fileInput.click();
  ui.dropzone.addEventListener("click", open);
  ui.dropzone.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      open();
    }
  });
  ui.fileInput.addEventListener("change", () => {
    const file = ui.fileInput.files?.[0];
    if (file) void handleFile(file);
  });

  for (const type of ["dragenter", "dragover"] as const) {
    ui.dropzone.addEventListener(type, (event) => {
      event.preventDefault();
      ui.dropzone.classList.add("is-dragging");
    });
  }
  for (const type of ["dragleave", "drop"] as const) {
    ui.dropzone.addEventListener(type, () => ui.dropzone.classList.remove("is-dragging"));
  }
  ui.dropzone.addEventListener("drop", (event) => {
    event.preventDefault();
    const file = event.dataTransfer?.files?.[0];
    if (file) void handleFile(file);
  });
}

function wireSample(): void {
  const apply = (): void => {
    const text = ui.sample.value.trim() || "Handgloves";
    const display = ui.preview.querySelector(".preview-line--display");
    if (display) display.textContent = text;
  };
  ui.sample.addEventListener("input", apply);
}

renderTargets();
wireDropzone();
wireSample();
ui.download.addEventListener("click", () => void runConversion());
setStatus("Ready — nothing leaves this tab.");
