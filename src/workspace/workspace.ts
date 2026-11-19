import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { bootAgentUi, newAgentTab, renderAgentTabs, setLlmKeySet } from "../agent-ui/agent";
import type { BrowserTab, WorkspaceState } from "../contracts/events";
import { subscribe } from "../chrome/router";
import { logError } from "../log";
import "./workspace.css";

const tabsEl = () => document.querySelector("#browser-tabs") as HTMLElement;
const urlInput = () => document.querySelector("#url-input") as HTMLInputElement;
const urlBar = () => document.querySelector(".url-bar") as HTMLElement;
const urlScheme = () => document.querySelector("#url-scheme") as HTMLElement;
const urlProgress = () => document.querySelector("#url-progress") as HTMLElement;
const slot = () => document.querySelector("#content-slot") as HTMLElement;
const navBack = () => document.querySelector("#nav-back") as HTMLButtonElement;
const navForward = () => document.querySelector("#nav-forward") as HTMLButtonElement;
const navReload = () => document.querySelector("#nav-reload") as HTMLButtonElement;

let lastJson = "";
let lastTabId = "";
let pendingAddressFocus = false;

function addressIsEditing(): boolean {
  return document.activeElement === urlInput();
}

function paintAddress(tab: BrowserTab | undefined): void {
  const address = urlInput();
  const scheme = urlScheme();
  const bar = urlBar();
  const progress = urlProgress();
  const back = navBack();
  const forward = navForward();
  const reload = navReload();
  if (!address || !scheme || !bar || !progress || !back || !forward || !reload) {
    return;
  }
  const loading = Boolean(tab?.loading);
  bar.classList.toggle("loading", loading);
  progress.hidden = !loading;
  back.disabled = !tab?.canGoBack;
  forward.disabled = !tab?.canGoForward;
  reload.disabled = !tab;
  reload.setAttribute("aria-label", loading ? "Stop" : "Reload");
  reload.title = loading ? "Stop" : "Reload";
  const href = tab?.url ?? "";
  scheme.textContent = href.startsWith("https:") ? "HTTPS" : href.startsWith("http:") ? "HTTP" : "";
  const tabChanged = (tab?.id ?? "") !== lastTabId;
  lastTabId = tab?.id ?? "";
  if (tabChanged || !addressIsEditing()) {
    address.value = href;
  }
  if (pendingAddressFocus && tab && !tab.url) {
    pendingAddressFocus = false;
    address.value = "";
    address.focus();
  }
}

function paintBrowserTabs(tabs: BrowserTab[], activeId: string | null): void {
  const strip = tabsEl();
  if (!strip) {
    return;
  }
  strip.replaceChildren();
  for (const tab of tabs) {
    const chip = document.createElement("div");
    chip.className = `tab-chip${tab.id === activeId ? " active" : ""}`;
    const title = document.createElement("button");
    title.type = "button";
    title.className = "tab-title";
    title.textContent = tab.title || tab.url;
    title.addEventListener("click", () => {
      void invoke("workspace_focus_tab", { id: tab.id });
    });
    chip.append(title);
    if (tabs.length > 1) {
      const close = document.createElement("button");
      close.type = "button";
      close.className = "tab-close";
      close.setAttribute("aria-label", "Close tab");
      close.textContent = "×";
      close.addEventListener("click", (event) => {
        event.stopPropagation();
        void invoke("workspace_close_tab", { id: tab.id });
      });
      chip.append(close);
    }
    strip.append(chip);
  }
  paintAddress(tabs.find((tab) => tab.id === activeId));
}

function paintWorkspace(state: WorkspaceState): void {
  const json = JSON.stringify(state);
  if (json === lastJson) {
    return;
  }
  lastJson = json;
  setLlmKeySet(state.llmKeySet);
  paintBrowserTabs(state.browserTabs, state.activeBrowserId);
  renderAgentTabs(state.agentTabs, state.activeAgentId);
}

async function refreshWorkspace(): Promise<void> {
  try {
    paintWorkspace(await invoke<WorkspaceState>("workspace_state"));
  } catch (error) {
    logError("workspace", error);
  }
}

function sendBounds(): void {
  const hole = slot();
  const chrome = document.querySelector(".browser-chrome");
  const workspace = document.querySelector("#view-workspace") as HTMLElement | null;
  if (!hole || !chrome || workspace?.hidden) {
    return;
  }
  document.documentElement.scrollTop = 0;
  document.body.scrollTop = 0;
  const holeRect = hole.getBoundingClientRect();
  const chromeRect = chrome.getBoundingClientRect();
  const address = document.querySelector(".url-bar");
  const addressBottom = address?.getBoundingClientRect().bottom ?? chromeRect.bottom;
  const top = Math.max(holeRect.top, chromeRect.bottom, addressBottom);
  const height = holeRect.bottom - top;
  if (holeRect.width < 48 || height < 48) {
    return;
  }
  void invoke("workspace_set_bounds", {
    x: holeRect.left,
    y: top,
    width: holeRect.width,
    height,
  }).catch((error: unknown) => {
    logError("workspace", error);
  });
}

function installAgentResize(): void {
  const workspace = document.querySelector(".workspace") as HTMLElement | null;
  const handle = document.querySelector("#agent-resize") as HTMLElement | null;
  const agents = document.querySelector(".workspace-agents") as HTMLElement | null;
  if (!workspace || !handle || !agents) {
    return;
  }
  const workspaceEl = workspace;
  const handleEl = handle;
  const agentsEl = agents;
  const minAgent = 220;
  const minBrowser = 280;
  let drag: { pointerId: number; startX: number; startWidth: number } | null = null;

  function clampWidth(next: number): number {
    const room = Math.max(
      minAgent,
      workspaceEl.getBoundingClientRect().width - minBrowser - handleEl.offsetWidth,
    );
    return Math.round(Math.min(room, Math.max(minAgent, next)));
  }

  function widthFor(clientX: number): number {
    if (!drag) {
      return minAgent;
    }
    return clampWidth(drag.startWidth + (drag.startX - clientX));
  }

  function applyWidth(next: number): void {
    workspaceEl.style.setProperty("--agent-width", `${next}px`);
    handleEl.setAttribute("aria-valuenow", String(next));
    sendBounds();
  }

  function finish(): void {
    if (!drag) {
      return;
    }
    drag = null;
    handleEl.classList.remove("dragging");
    void invoke("workspace_set_page_covered", { covered: false }).finally(sendBounds);
  }

  handle.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    drag = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: agentsEl.getBoundingClientRect().width,
    };
    handle.classList.add("dragging");
    try {
      handle.setPointerCapture(event.pointerId);
    } catch (error) {
      logError("workspace", error);
    }
    void invoke("workspace_set_page_covered", { covered: true });
  });
  handle.addEventListener("pointermove", (event) => {
    if (!drag || event.pointerId !== drag.pointerId) {
      return;
    }
    applyWidth(widthFor(event.clientX));
  });
  handle.addEventListener("pointerup", finish);
  handle.addEventListener("pointercancel", finish);
  handle.addEventListener("keydown", (event) => {
    const current = agentsEl.getBoundingClientRect().width;
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      applyWidth(clampWidth(current + 24));
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      applyWidth(clampWidth(current - 24));
    }
  });
}

function installAgentCollapse(): void {
  const workspace = document.querySelector(".workspace") as HTMLElement | null;
  const collapse = document.querySelector("#collapse-agents") as HTMLButtonElement | null;
  const open = document.querySelector("#open-agents") as HTMLButtonElement | null;
  if (!workspace || !collapse || !open) {
    return;
  }
  const workspaceEl = workspace;
  const collapseEl = collapse;
  const openEl = open;

  function apply(collapsed: boolean): void {
    workspaceEl.classList.toggle("agents-collapsed", collapsed);
    collapseEl.setAttribute("aria-expanded", collapsed ? "false" : "true");
    openEl.setAttribute("aria-expanded", collapsed ? "false" : "true");
    openEl.hidden = !collapsed;
    try {
      localStorage.setItem("buho-agents-collapsed", collapsed ? "1" : "0");
    } catch (error) {
      logError("workspace", error);
    }
    sendBounds();
    window.requestAnimationFrame(sendBounds);
  }

  collapseEl.addEventListener("click", () => apply(true));
  openEl.addEventListener("click", () => apply(false));
  let collapsed = false;
  try {
    collapsed = localStorage.getItem("buho-agents-collapsed") === "1";
  } catch (error) {
    logError("workspace", error);
  }
  apply(collapsed);
}

export function bootWorkspace(): void {
  bootAgentUi();
  document.querySelector("#url-form")?.addEventListener("submit", (event) => {
    event.preventDefault();
    const url = urlInput()?.value.trim();
    if (!url) {
      return;
    }
    urlInput()?.blur();
    void invoke("workspace_navigate", { url }).catch((error: unknown) => {
      logError("workspace", error);
    });
  });
  urlInput()?.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      lastJson = "";
      void refreshWorkspace().then(() => urlInput()?.blur());
    }
  });
  navBack()?.addEventListener("click", () => {
    void invoke("workspace_history", { direction: "back" });
  });
  navForward()?.addEventListener("click", () => {
    void invoke("workspace_history", { direction: "forward" });
  });
  navReload()?.addEventListener("click", () => {
    const loading = urlBar()?.classList.contains("loading");
    void invoke("workspace_history", { direction: loading ? "stop" : "reload" });
  });
  document.querySelector("#new-browser-tab")?.addEventListener("click", () => {
    pendingAddressFocus = true;
    void invoke<WorkspaceState>("workspace_new_tab")
      .then((state) => {
        lastJson = "";
        paintWorkspace(state);
      })
      .catch((error: unknown) => {
        pendingAddressFocus = false;
        logError("workspace", error);
      });
  });
  document.querySelector("#new-agent-tab")?.addEventListener("click", () => {
    newAgentTab();
  });
  subscribe((route) => {
    if (route.name !== "workspace") {
      return;
    }
    const instanceId = route.instanceId;
    const opened = instanceId
      ? invoke<WorkspaceState>("workspace_open_instance", { instanceId })
      : invoke<WorkspaceState>("workspace_show");
    void opened
      .then((state) => {
        if (!instanceId) {
          const tab = state.browserTabs.find((item) => item.id === state.activeBrowserId);
          if (tab && !tab.url) {
            pendingAddressFocus = true;
          }
        }
        lastJson = "";
        paintWorkspace(state);
        sendBounds();
        window.requestAnimationFrame(sendBounds);
      })
      .catch((error: unknown) => {
        logError("workspace", error);
      });
  });
  void listen<WorkspaceState>("workspace-updated", (event) => {
    paintWorkspace(event.payload);
  }).catch((error: unknown) => {
    logError("workspace", error);
  });
  installAgentResize();
  installAgentCollapse();
  window.addEventListener("resize", sendBounds);
  window.addEventListener("scroll", sendBounds, true);
  const hole = slot();
  const chrome = document.querySelector(".browser-chrome");
  const watcher = new ResizeObserver(sendBounds);
  if (hole) {
    watcher.observe(hole);
  }
  if (chrome) {
    watcher.observe(chrome);
  }
  void document.fonts?.ready.then(sendBounds);
}
