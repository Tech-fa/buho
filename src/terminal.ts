import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { logError } from "./log";

type TerminalInfo = {
  shell: string;
  apiUrl: string | null;
  mcpUrl: string | null;
};

type DockView = {
  panel: number;
  windowHeight: number;
  collapsed: boolean;
  dragging: boolean;
};

const status = document.querySelector("#status") as HTMLParagraphElement;
const mount = document.querySelector("#term") as HTMLDivElement;
const pull = document.querySelector("#pull") as HTMLDivElement;
const pullLabel = document.querySelector("#pull-label") as HTMLSpanElement;
const panel = document.querySelector("#panel") as HTMLDivElement;

const term = new Terminal({
  cursorBlink: true,
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
  fontSize: 13,
  theme: {
    background: "#1c1915",
    foreground: "#f9fafb",
    cursor: "#60a5fa",
  },
});
const fit = new FitAddon();
term.loadAddon(fit);
term.open(mount);

let dock: DockView = {
  panel: 36,
  windowHeight: 760,
  collapsed: true,
  dragging: false,
};
let ptyStarted = false;
let requestId = 0;
let drag: { startY: number; startPanel: number; moved: boolean } | null = null;
let pending: { height: number; dragging: boolean; snap: boolean } | null = null;
let frame = 0;

function writeOutput(payload: string): void {
  const binary = atob(payload);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  term.write(bytes);
}

function describe(info: TerminalInfo): string {
  const parts = [info.shell];
  if (info.mcpUrl) {
    parts.push(`APP_MCP_URL=${info.mcpUrl}`);
  }
  if (info.apiUrl) {
    parts.push(`API_URL=${info.apiUrl}`);
  }
  return parts.join(" · ");
}

function preview(height: number, dragging: boolean): void {
  paint(
    {
      panel: Math.max(36, height),
      windowHeight: dock.windowHeight,
      collapsed: height <= 36.5,
      dragging,
    },
    false,
  );
}

function paint(next: DockView, focus: boolean): void {
  const opened = dock.collapsed && !next.collapsed;
  dock = next;
  panel.style.setProperty("--panel-height", next.dragging ? `${next.panel}px` : "100%");
  pull.setAttribute("aria-valuenow", String(Math.round(next.panel)));
  pull.setAttribute(
    "aria-valuemax",
    String(Math.max(36, Math.round(next.windowHeight - 180))),
  );
  pullLabel.textContent = next.collapsed ? "Pull up the terminal" : "Terminal";
  pull.setAttribute(
    "aria-label",
    next.collapsed ? "Pull up the terminal" : "Pull the terminal down",
  );
  const showTerm = !next.collapsed;
  status.hidden = !showTerm;
  mount.hidden = !showTerm;
  if (!showTerm) {
    return;
  }
  requestAnimationFrame(() => {
    fit.fit();
    if (!ptyStarted) {
      void startPty(focus || opened);
      return;
    }
    if (focus || opened) {
      term.focus();
    }
  });
}

async function startPty(focus: boolean): Promise<void> {
  if (ptyStarted) {
    return;
  }
  ptyStarted = true;
  status.hidden = false;
  status.textContent = "Starting the terminal…";
  try {
    fit.fit();
    const info = await invoke<TerminalInfo>("terminal_start", {
      rows: term.rows,
      cols: term.cols,
    });
    status.textContent = describe(info);
    if (focus) {
      term.focus();
    }
  } catch (error) {
    ptyStarted = false;
    logError("terminal", error);
    status.textContent = String(error);
  }
}

async function commit(height: number, dragging: boolean, snap: boolean): Promise<void> {
  const id = ++requestId;
  try {
    const view = await invoke<DockView>("set_preview_terminal", { height, dragging, snap });
    if (id === requestId) {
      paint(view, false);
    }
  } catch (error) {
    logError("terminal", error);
    if (id === requestId) {
      status.hidden = false;
      status.textContent = String(error);
    }
  }
}

function schedule(height: number, dragging: boolean, snap: boolean): void {
  pending = { height, dragging, snap };
  if (frame) {
    return;
  }
  frame = requestAnimationFrame(() => {
    frame = 0;
    const next = pending;
    pending = null;
    if (next) {
      void commit(next.height, next.dragging, next.snap);
    }
  });
}

async function toggle(): Promise<void> {
  const id = ++requestId;
  try {
    const view = await invoke<DockView>("toggle_preview_terminal");
    if (id === requestId) {
      paint(view, false);
    }
  } catch (error) {
    logError("terminal", error);
    status.hidden = false;
    status.textContent = String(error);
  }
}

function cancelScheduled(): void {
  if (frame) {
    cancelAnimationFrame(frame);
    frame = 0;
  }
  pending = null;
}

function endDrag(event: PointerEvent): void {
  if (!drag) {
    return;
  }
  const next = drag.startPanel + (drag.startY - event.screenY);
  const moved = drag.moved;
  drag = null;
  cancelScheduled();
  if (!moved) {
    void toggle();
    return;
  }
  void commit(next, false, true);
}

pull.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) {
    return;
  }
  event.preventDefault();
  drag = { startY: event.screenY, startPanel: dock.panel, moved: false };
  try {
    pull.setPointerCapture(event.pointerId);
  } catch (error) {
    logError("terminal", error);
  }
  preview(dock.panel, true);
  schedule(dock.panel, true, false);
});

window.addEventListener("pointermove", (event) => {
  if (!drag) {
    return;
  }
  const next = drag.startPanel + (drag.startY - event.screenY);
  if (Math.abs(event.screenY - drag.startY) > 4) {
    drag.moved = true;
  }
  preview(next, true);
  schedule(next, true, false);
});

window.addEventListener("pointerup", endDrag);
window.addEventListener("pointercancel", (event) => {
  if (!drag) {
    return;
  }
  drag = null;
  cancelScheduled();
  void commit(dock.panel, false, true);
  event.preventDefault();
});

pull.addEventListener("keydown", (event) => {
  if (event.key === "ArrowUp") {
    event.preventDefault();
    void commit(dock.panel + 80, false, false);
  } else if (event.key === "ArrowDown") {
    event.preventDefault();
    void commit(dock.panel - 80, false, false);
  } else if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    void toggle();
  } else if (event.key === "Escape" && drag) {
    const start = drag.startPanel;
    drag = null;
    cancelScheduled();
    preview(start, false);
    void commit(start, false, true);
  }
});

window.addEventListener("blur", () => {
  if (!drag) {
    return;
  }
  const next = dock.panel;
  drag = null;
  cancelScheduled();
  void commit(next, false, true);
});

new ResizeObserver(() => {
  if (!mount.hidden) {
    fit.fit();
  }
}).observe(mount);

void listen<string>("terminal-output", (event) => {
  writeOutput(event.payload);
}).catch((error: unknown) => {
  logError("terminal", error);
});

void listen<string>("terminal-exit", (event) => {
  ptyStarted = false;
  status.hidden = false;
  status.textContent = event.payload;
}).catch((error: unknown) => {
  logError("terminal", error);
});

term.onData((data) => {
  void invoke("terminal_write", { data });
});

term.onResize(({ cols, rows }) => {
  void invoke("terminal_resize", { cols, rows });
});
