import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { logError } from "../log";
import { openBrowser, openFourssSetup, openFourssWorkspace } from "../chrome/shell";
import type { AppInstance, CatalogView, InstanceStatus } from "../contracts/catalog";
import "./home.css";

const root = document.querySelector("#view-home") as HTMLElement;

function labelFor(status: InstanceStatus): string {
  if (status === "running") return "Running";
  if (status === "deploying") return "Setting up";
  if (status === "error") return "Needs attention";
  return "Set up";
}

function hasAddress(instance: AppInstance): boolean {
  return Boolean(instance.appUrl);
}

function actionFor(instance: AppInstance): string {
  return hasAddress(instance) ? "Open" : "Set up";
}

function card(instance: AppInstance, summary: string): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "app-card";
  const copy = document.createElement("span");
  const title = document.createElement("h2");
  title.textContent = instance.name;
  const detail = document.createElement("p");
  detail.textContent = summary;
  copy.append(title, detail);
  const pill = document.createElement("span");
  pill.className = `status-pill ${instance.status}`;
  pill.textContent = `${labelFor(instance.status)} · ${actionFor(instance)}`;
  button.append(copy, pill);
  button.addEventListener("click", () => {
    if (hasAddress(instance)) {
      openFourssWorkspace();
    } else {
      openFourssSetup();
    }
  });
  return button;
}

function render(view: CatalogView): void {
  if (!root) {
    return;
  }
  const product = view.products[0];
  const instance = view.instances[0];
  root.replaceChildren();
  const page = document.createElement("div");
  page.className = "home";
  const header = document.createElement("header");
  const eyebrow = document.createElement("p");
  eyebrow.className = "eyebrow";
  eyebrow.textContent = "Apps";
  const heading = document.createElement("h1");
  heading.textContent = "Your apps";
  const intro = document.createElement("p");
  intro.textContent = "Open the browser on its own, or open an app to set it up.";
  const browser = document.createElement("button");
  browser.type = "button";
  browser.className = "home-browser";
  browser.textContent = "Open browser";
  browser.addEventListener("click", () => openBrowser());
  header.append(eyebrow, heading, intro, browser);
  const list = document.createElement("div");
  list.className = "home-list";
  if (instance && product) {
    list.append(card(instance, product.summary));
  }
  page.append(header, list);
  root.append(page);
}

export async function refreshCatalog(): Promise<void> {
  try {
    render(await invoke<CatalogView>("catalog_view"));
  } catch (error) {
    logError("apps", error);
  }
}

export function bootHome(): void {
  void refreshCatalog();
  void listen("catalog-updated", () => {
    void refreshCatalog();
  }).catch((error: unknown) => {
    logError("apps", error);
  });
  void listen("deploy-status", () => {
    void refreshCatalog();
  }).catch((error: unknown) => {
    logError("apps", error);
  });
}
