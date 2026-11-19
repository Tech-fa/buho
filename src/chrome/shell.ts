import { invoke } from "@tauri-apps/api/core";
import { FOURSS_INSTANCE_ID, type Route } from "../contracts/catalog";
import { getRoute, setRoute, subscribe } from "./router";
import "./chrome.css";

const home = document.querySelector("#view-home") as HTMLElement;
const setup = document.querySelector("#view-setup") as HTMLElement;
const workspace = document.querySelector("#view-workspace") as HTMLElement;
const back = document.querySelector("#back-apps") as HTMLButtonElement;
const browser = document.querySelector("#open-browser") as HTMLButtonElement;

function paint(route: Route): void {
  if (!home || !setup || !workspace || !back || !browser) {
    return;
  }
  home.hidden = route.name !== "home";
  setup.hidden = route.name !== "setup";
  workspace.hidden = route.name !== "workspace";
  back.hidden = route.name === "home";
  browser.hidden = route.name === "workspace";
  document.body.classList.toggle("workspace-open", route.name === "workspace");
  if (route.name !== "workspace") {
    void invoke("workspace_hide").catch(() => undefined);
  }
}

export function bootChrome(): void {
  back?.addEventListener("click", () => setRoute({ name: "home" }));
  browser?.addEventListener("click", () => openBrowser());
  subscribe(paint);
}

export function openBrowser(): void {
  setRoute({ name: "workspace" });
}

export function openFourssSetup(): void {
  setRoute({ name: "setup", instanceId: FOURSS_INSTANCE_ID });
}

export function openFourssWorkspace(): void {
  setRoute({ name: "workspace", instanceId: FOURSS_INSTANCE_ID });
}

export { getRoute, setRoute, subscribe };
