import { invoke } from "@tauri-apps/api/core";
import { logError } from "../log";
import { open } from "@tauri-apps/plugin-dialog";
import type { Field, SaveOutcome, ShellState } from "./types";

const appName = document.querySelector("#app-name") as HTMLHeadingElement;
const summary = document.querySelector("#summary") as HTMLParagraphElement;
const uiPath = document.querySelector("#ui-path") as HTMLInputElement;
const appUrlInput = document.querySelector("#app-url") as HTMLInputElement;
const urlError = document.querySelector("#url-error") as HTMLParagraphElement;
const mcp = document.querySelector("#mcp") as HTMLParagraphElement;
const pathError = document.querySelector("#path-error") as HTMLParagraphElement;
const fields = document.querySelector("#fields") as HTMLDivElement;
const addRow = document.querySelector("#add-row") as HTMLDivElement;
const newKey = document.querySelector("#new-key") as HTMLInputElement;
const formError = document.querySelector("#form-error") as HTMLParagraphElement;
const browseButton = document.querySelector("#browse") as HTMLButtonElement;
const saveButton = document.querySelector("#save") as HTMLButtonElement;
const openButton = document.querySelector("#open") as HTMLButtonElement;
const server = document.querySelector("#server") as HTMLParagraphElement;
const storage = document.querySelector("#storage") as HTMLParagraphElement;
const form = document.querySelector("#config-form") as HTMLFormElement;

let state: ShellState | null = null;
let fieldErrors: Record<string, string> = {};
let afterRender: () => void = () => undefined;

function text(value: string): Text {
  return document.createTextNode(value);
}

function summaryText(next: ShellState): string {
  if (next.uiSource === "none") {
    const lead = "Enter the app URL. Agent CLI can then move through that app and edit the form that is open.";
    return next.status ? `${next.status} ${lead}` : lead;
  }
  const parts: string[] = [];
  if (next.manifestFound) {
    parts.push("Settings come from shell.manifest.json in this bundle.");
  } else {
    parts.push("This bundle has no shell.manifest.json. Add one to name the settings, or enter them here.");
  }
  parts.push(`The app reads /${next.configFile} from this window's server.`);
  if (next.status) {
    parts.push(next.status);
  }
  return parts.join(" ");
}

function renderField(field: Field, value: unknown): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "field";
  const inputId = `field-${field.key}`;
  if (field.type === "boolean") {
    const label = document.createElement("label");
    label.className = "check";
    label.htmlFor = inputId;
    const input = document.createElement("input");
    input.id = inputId;
    input.type = "checkbox";
    input.dataset.key = field.key;
    input.checked = value === true;
    label.append(input, text(field.label));
    wrap.append(label);
  } else {
    const label = document.createElement("label");
    label.htmlFor = inputId;
    label.textContent = field.required ? `${field.label} *` : field.label;
    wrap.append(label);
    const input = field.type === "text" ? document.createElement("textarea") : document.createElement("input");
    input.id = inputId;
    input.dataset.key = field.key;
    input.placeholder = field.placeholder;
    input.spellcheck = false;
    if (input instanceof HTMLInputElement) {
      input.type = field.type === "number" ? "number" : "text";
      input.autocomplete = "off";
    }
    if (typeof value === "string" || typeof value === "number") {
      input.value = String(value);
    }
    wrap.append(input);
  }
  if (field.description) {
    const hint = document.createElement("p");
    hint.className = "hint";
    hint.textContent = field.description;
    wrap.append(hint);
  }
  const error = fieldErrors[field.key];
  if (error) {
    const message = document.createElement("p");
    message.className = "field-error";
    message.textContent = error;
    wrap.append(message);
  }
  if (state && !state.manifestFound) {
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "remove";
    remove.textContent = "Remove";
    remove.addEventListener("click", () => removeField(field.key));
    wrap.append(remove);
  }
  return wrap;
}

function readValues(): Record<string, unknown> {
  const values: Record<string, unknown> = {};
  if (!state) {
    return values;
  }
  for (const field of state.fields) {
    const input = fields.querySelector<HTMLInputElement | HTMLTextAreaElement>(
      `[data-key="${CSS.escape(field.key)}"]`,
    );
    if (!input) {
      continue;
    }
    if (field.type === "boolean" && input instanceof HTMLInputElement) {
      values[field.key] = input.checked;
      continue;
    }
    const raw = input.value.trim();
    if (raw === "") {
      continue;
    }
    if (field.type === "number") {
      const number = Number(raw);
      values[field.key] = Number.isFinite(number) ? number : raw;
      continue;
    }
    values[field.key] = input.value.trim();
  }
  return values;
}

function removeField(key: string): void {
  if (!state) {
    return;
  }
  const values = readValues();
  delete values[key];
  state = {
    ...state,
    fields: state.fields.filter((field) => field.key !== key),
    values,
  };
  render(state);
}

export function render(next: ShellState): void {
  state = next;
  const choosing = next.uiSource === "none";
  appName.textContent = choosing ? "Open an app" : next.appName || "Web app";
  summary.textContent = summaryText(next);
  if (document.activeElement !== appUrlInput) {
    appUrlInput.value = next.appUrl;
  }
  mcp.textContent = next.mcpUrl ? `Agent tools: ${next.mcpUrl}` : "";
  uiPath.value = choosing ? "" : next.uiPath;
  browseButton.textContent = choosing ? "Choose bundle" : "Change";
  browseButton.classList.toggle("primary", choosing);
  form.hidden = choosing;
  server.hidden = choosing;
  storage.hidden = choosing;
  addRow.hidden = choosing || next.manifestFound;
  openButton.disabled = !next.readyToOpen;
  server.textContent = `Served at ${next.serverUrl}`;
  storage.textContent = next.appId ? `Saved settings for ${next.appId} live in ${next.dataDir}` : "";
  fields.replaceChildren();
  if (!choosing) {
    for (const field of next.fields) {
      fields.append(renderField(field, next.values[field.key]));
    }
  }
  afterRender();
}

async function loadPath(path: string): Promise<void> {
  pathError.textContent = "";
  try {
    const next = await invoke<ShellState>("set_ui_path", { path });
    fieldErrors = {};
    formError.textContent = "";
    render(next);
  } catch (error) {
    logError("web app folder", error);
    pathError.textContent = String(error);
  }
}

async function saveAndMaybeOpen(openAfter: boolean): Promise<void> {
  if (!state) {
    return;
  }
  formError.textContent = "";
  saveButton.disabled = true;
  try {
    const outcome = await invoke<SaveOutcome>("save_config", { values: readValues() });
    fieldErrors = outcome.fieldErrors;
    formError.textContent = outcome.message;
    render(outcome.state);
    if (openAfter && Object.keys(outcome.fieldErrors).length === 0 && !outcome.message) {
      await invoke("open_served_app");
    }
  } catch (error) {
    logError("save settings", error);
    formError.textContent = String(error);
  } finally {
    saveButton.disabled = false;
  }
}

export function mountBundle(hooks: { afterRender: () => void }): void {
  afterRender = hooks.afterRender;

  document.querySelector("#browse")?.addEventListener("click", async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Choose the web app bundle",
    });
    if (typeof selected === "string") {
      await loadPath(selected);
    }
  });

  document.querySelector("#load")?.addEventListener("click", async () => {
    await loadPath(uiPath.value.trim());
  });

  document.querySelector("#add-setting")?.addEventListener("click", () => {
    if (!state) {
      return;
    }
    const key = newKey.value.trim();
    if (!/^[A-Za-z][A-Za-z0-9_-]*$/.test(key)) {
      formError.textContent = "Setting names start with a letter and use letters, numbers, _ or -.";
      return;
    }
    if (state.fields.some((field) => field.key === key)) {
      formError.textContent = `${key} is already in the list.`;
      return;
    }
    formError.textContent = "";
    const values = readValues();
    values[key] = "";
    state = {
      ...state,
      fields: [
        ...state.fields,
        { key, label: key, description: "", type: "string", required: false, placeholder: "" },
      ],
      values,
    };
    newKey.value = "";
    render(state);
  });

  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void saveAndMaybeOpen(true);
  });

  document.querySelector("#cli")?.addEventListener("click", () => {
    void invoke("open_agent_cli");
  });

  document.querySelector("#open-url")?.addEventListener("click", async () => {
    urlError.textContent = "";
    try {
      render(await invoke<ShellState>("open_public_app", { url: appUrlInput.value.trim() }));
    } catch (error) {
      logError("open the app address", error);
      urlError.textContent = String(error);
    }
  });

  openButton.addEventListener("click", async () => {
    formError.textContent = "";
    try {
      await invoke("open_served_app");
    } catch (error) {
      logError("open the app", error);
      formError.textContent = String(error);
    }
  });
}

export async function loadShellState(): Promise<ShellState> {
  return invoke<ShellState>("shell_state");
}
