import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { logError } from "../log";
import type { AgentEvent, AgentTab } from "../contracts/events";

type Provider = "auto" | "anthropic" | "openai" | "deepseek";

type StoredPart = {
  type?: string;
  text?: string;
  name?: string;
  input?: unknown;
  content?: unknown;
};

type StoredMessage = {
  role?: string;
  content?: string | StoredPart[];
};

type ChatFile = {
  provider?: string;
  model?: string;
  messages?: StoredMessage[];
};

type ChatSummary = {
  id: string;
  sessionId: string;
  title: string;
  provider: string;
  model: string;
  open: boolean;
  updatedAt: number;
};

type Pane = {
  tab: AgentTab;
  root: HTMLElement;
  transcript: HTMLElement;
  composer: HTMLTextAreaElement;
  send: HTMLButtonElement;
  attach: HTMLButtonElement;
  provider: HTMLSelectElement;
  files: string[];
  fileRow: HTMLElement;
  gate: HTMLElement;
  running: boolean;
  watch: number | null;
  loaded: boolean;
  pending: AgentEvent[];
};

const PROVIDER_KEY = "buho-llm-provider";

const panes = new Map<string, Pane>();
let activeId = "";
let llmKeySet = false;

export function setLlmKeySet(value: boolean): void {
  llmKeySet = value;
  for (const pane of panes.values()) {
    const banner = pane.root.querySelector(".key-missing") as HTMLElement | null;
    if (banner && !banner.dataset.forced) {
      banner.hidden = value;
    }
  }
}

function bubble(kind: string, text: string): HTMLParagraphElement {
  const node = document.createElement("p");
  node.className = `bubble ${kind}`;
  node.textContent = text;
  return node;
}

function thinkingBlock(pane: Pane, keepOpen: boolean): HTMLDetailsElement {
  const last = pane.transcript.lastElementChild;
  if (last instanceof HTMLDetailsElement && last.classList.contains("thinking") && last.dataset.openRun === "1") {
    return last;
  }
  const details = document.createElement("details");
  details.className = "thinking";
  if (keepOpen) {
    details.dataset.openRun = "1";
    details.open = true;
  }
  const summary = document.createElement("summary");
  summary.textContent = "Thinking";
  const body = document.createElement("div");
  body.className = "thinking-body";
  details.append(summary, body);
  append(pane, details);
  return details;
}

function addThinkingStep(pane: Pane, text: string): void {
  const line = document.createElement("p");
  line.className = "thinking-step";
  line.textContent = text;
  thinkingBlock(pane, true).querySelector(".thinking-body")?.append(line);
}

function closeThinking(pane: Pane): void {
  const last = pane.transcript.lastElementChild;
  if (last instanceof HTMLDetailsElement && last.classList.contains("thinking")) {
    delete last.dataset.openRun;
    last.open = false;
  }
}

function append(pane: Pane, node: HTMLElement): void {
  pane.transcript.append(node);
  pane.transcript.scrollTop = pane.transcript.scrollHeight;
}

function savedProvider(): Provider {
  const value = localStorage.getItem(PROVIDER_KEY);
  return value === "anthropic" || value === "openai" || value === "deepseek" ? value : "auto";
}

function fileName(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

function messageText(text: string, files: string[]): string {
  if (files.length === 0) {
    return text;
  }
  const list = files.map((path) => `- ${path}`).join("\n");
  return text ? `${text}\n\nAttached files:\n${list}` : `Attached files:\n${list}`;
}

function paintFiles(pane: Pane): void {
  pane.fileRow.replaceChildren();
  pane.fileRow.hidden = pane.files.length === 0;
  for (const path of pane.files) {
    const chip = document.createElement("span");
    chip.className = "composer-file";
    const name = document.createElement("span");
    name.textContent = fileName(path);
    name.title = path;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.setAttribute("aria-label", `Remove ${fileName(path)}`);
    remove.textContent = "×";
    remove.addEventListener("click", () => {
      pane.files = pane.files.filter((item) => item !== path);
      paintFiles(pane);
      refreshSend(pane);
    });
    chip.append(name, remove);
    pane.fileRow.append(chip);
  }
}

function grow(composer: HTMLTextAreaElement): void {
  composer.style.height = "auto";
  composer.style.height = `${Math.min(composer.scrollHeight, 128)}px`;
}

function refreshSend(pane: Pane): void {
  const empty = pane.composer.value.trim().length === 0 && pane.files.length === 0;
  pane.send.disabled = pane.running ? false : empty;
  pane.send.classList.toggle("stopping", pane.running);
  pane.send.setAttribute("aria-label", pane.running ? "Stop" : "Send");
  pane.send.title = pane.running ? "Stop" : "Send";
}

function setRunning(pane: Pane, running: boolean): void {
  pane.running = running;
  pane.composer.disabled = running;
  pane.attach.disabled = running;
  pane.provider.disabled = running;
  refreshSend(pane);
  if (pane.watch !== null) {
    window.clearTimeout(pane.watch);
    pane.watch = null;
  }
  if (running) {
    pane.watch = window.setTimeout(() => {
      pane.watch = null;
      if (!pane.running) {
        return;
      }
      append(pane, bubble("assistant", "The agent did not finish. Stop it and try again."));
      setRunning(pane, false);
    }, 180_000);
  }
}

function paneFor(event: AgentEvent): Pane | undefined {
  const exact = [...panes.values()].find((item) => item.tab.sessionId === event.sessionId);
  if (exact) {
    return exact;
  }
  if (event.type === "gate.ask") {
    return (activeId && panes.get(activeId)) || [...panes.values()][0];
  }
  return undefined;
}

export function renderAgentTabs(tabs: AgentTab[], active: string | null): void {
  const strip = document.querySelector("#agent-tabs") as HTMLElement | null;
  const host = document.querySelector("#agent-panes") as HTMLElement | null;
  if (!strip || !host) {
    return;
  }
  strip.replaceChildren();
  for (const tab of tabs) {
    const chip = document.createElement("div");
    chip.className = `tab-chip${tab.id === active ? " active" : ""}`;
    const title = document.createElement("button");
    title.type = "button";
    title.className = "tab-title";
    title.textContent = tab.title || "New chat";
    title.addEventListener("click", () => {
      void invoke("workspace_focus_agent", { id: tab.id });
    });
    const close = document.createElement("button");
    close.type = "button";
    close.className = "tab-close";
    close.setAttribute("aria-label", "Close chat");
    close.textContent = "×";
    close.addEventListener("click", (event) => {
      event.stopPropagation();
      void invoke("workspace_close_agent", { id: tab.id }).catch((error: unknown) => {
        logError("agent", error);
      });
    });
    chip.append(title, close);
    strip.append(chip);
    const existing = panes.get(tab.id);
    if (existing) {
      existing.tab = tab;
    } else {
      const pane = createPane(host, tab);
      panes.set(tab.id, pane);
      void loadTranscript(pane);
    }
  }
  for (const [id, pane] of panes) {
    if (!tabs.some((tab) => tab.id === id)) {
      if (pane.watch !== null) {
        window.clearTimeout(pane.watch);
      }
      pane.root.remove();
      panes.delete(id);
    }
  }
  activeId = active || "";
  let empty = host.querySelector(".agent-empty") as HTMLElement | null;
  if (!empty) {
    empty = document.createElement("p");
    empty.className = "agent-empty";
    empty.textContent = "No chat open. Press + for a new one, or open one from History.";
    host.append(empty);
  }
  empty.hidden = tabs.length > 0;
  for (const [id, pane] of panes) {
    pane.root.hidden = id !== activeId;
  }
  void paintHistory();
}

function createPane(host: HTMLElement, tab: AgentTab): Pane {
  const root = document.createElement("div");
  root.className = "agent-pane";
  root.dataset.agentId = tab.id;
  const missing = document.createElement("p");
  missing.className = "key-missing";
  missing.hidden = llmKeySet;
  missing.textContent = "Set BUHO_LLM_KEY to talk to the model.";
  const transcript = document.createElement("div");
  transcript.className = "transcript";
  const gate = document.createElement("div");
  gate.className = "gate-card";
  gate.hidden = true;
  const composer = document.createElement("textarea");
  composer.rows = 1;
  composer.placeholder = "Ask the agent…";
  const fileRow = document.createElement("div");
  fileRow.className = "composer-files";
  fileRow.hidden = true;
  const attach = document.createElement("button");
  attach.type = "button";
  attach.className = "composer-icon";
  attach.setAttribute("aria-label", "Attach file");
  attach.title = "Attach file";
  attach.innerHTML =
    '<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6.1 11.6 11 6.7a2.1 2.1 0 0 0-3-3L3.4 8.4a3.2 3.2 0 0 0 4.5 4.5l5.1-5.1" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/></svg>';
  const provider = document.createElement("select");
  provider.className = "composer-provider";
  provider.setAttribute("aria-label", "Model");
  provider.innerHTML =
    '<option value="auto">Auto</option><option value="anthropic">Claude</option><option value="openai">GPT-4o</option><option value="deepseek">DeepSeek</option>';
  provider.value = savedProvider();
  const send = document.createElement("button");
  send.type = "button";
  send.className = "composer-send";
  send.disabled = true;
  send.innerHTML =
    '<svg class="icon-send" viewBox="0 0 16 16" aria-hidden="true"><path d="M8 12.2V3.8M4.6 7.2 8 3.8l3.4 3.4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg><svg class="icon-stop" viewBox="0 0 16 16" aria-hidden="true"><rect x="5" y="5" width="6" height="6" rx="1" fill="currentColor"/></svg>';
  const bar = document.createElement("div");
  bar.className = "composer-bar";
  bar.append(attach, provider, send);
  const row = document.createElement("div");
  row.className = "composer";
  row.append(fileRow, composer, bar);
  root.append(missing, transcript, gate, row);
  host.append(root);
  const pane: Pane = {
    tab,
    root,
    transcript,
    composer,
    send,
    attach,
    provider,
    files: [],
    fileRow,
    gate,
    running: false,
    watch: null,
    loaded: false,
    pending: [],
  };
  refreshSend(pane);
  attach.addEventListener("click", () => void attachFiles(pane));
  provider.addEventListener("change", () => {
    const choice =
      provider.value === "anthropic" || provider.value === "openai" || provider.value === "deepseek"
        ? provider.value
        : "auto";
    localStorage.setItem(PROVIDER_KEY, choice);
    for (const other of panes.values()) {
      other.provider.value = choice;
    }
  });
  send.addEventListener("click", () => {
    if (pane.running) {
      void invoke("agent_stop", { sessionId: pane.tab.sessionId }).catch((error: unknown) => {
        logError("agent", error);
        setRunning(pane, false);
      });
      return;
    }
    void submit(pane);
  });
  composer.addEventListener("input", () => {
    grow(composer);
    refreshSend(pane);
  });
  composer.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void submit(pane);
    }
  });
  return pane;
}

async function attachFiles(pane: Pane): Promise<void> {
  try {
    const selected = await open({ multiple: true, title: "Attach files" });
    const paths = typeof selected === "string" ? [selected] : selected ?? [];
    for (const path of paths) {
      if (!pane.files.includes(path)) {
        pane.files.push(path);
      }
    }
    paintFiles(pane);
    refreshSend(pane);
  } catch (error) {
    logError("agent", error);
  }
}

async function submit(pane: Pane): Promise<void> {
  const typed = pane.composer.value.trim();
  const files = pane.files.slice();
  if (pane.running || (typed.length === 0 && files.length === 0)) {
    return;
  }
  const text = messageText(typed, files);
  pane.composer.value = "";
  pane.files = [];
  grow(pane.composer);
  paintFiles(pane);
  refreshSend(pane);
  append(pane, bubble("user", text));
  setRunning(pane, true);
  try {
    const outcome = await invoke<{ ok: boolean; message?: string }>("agent_send", {
      sessionId: pane.tab.sessionId,
      text,
      provider: pane.provider.value,
    });
    if (outcome && outcome.ok === false) {
      const banner = pane.root.querySelector(".key-missing") as HTMLElement | null;
      if (banner && outcome.message) {
        banner.hidden = false;
        banner.dataset.forced = "1";
        banner.textContent = outcome.message;
      }
      if (outcome.message) {
        append(pane, bubble("assistant", outcome.message));
      }
      setRunning(pane, false);
    }
  } catch (error) {
    logError("agent", error);
    append(pane, bubble("assistant", String(error)));
    setRunning(pane, false);
  }
}

function providerLabel(provider: string): string {
  if (provider === "anthropic") return "Claude";
  if (provider === "openai") return "GPT-4o";
  if (provider === "deepseek") return "DeepSeek";
  return "Auto";
}

function paintStored(pane: Pane, messages: StoredMessage[]): void {
  const steps: string[] = [];
  const flush = () => {
    if (steps.length === 0) {
      return;
    }
    const block = thinkingBlock(pane, false);
    const body = block.querySelector(".thinking-body");
    for (const step of steps) {
      const line = document.createElement("p");
      line.className = "thinking-step";
      line.textContent = step;
      body?.append(line);
    }
    steps.length = 0;
  };
  for (const message of messages) {
    const content = message.content;
    if (typeof content === "string") {
      flush();
      append(pane, bubble(message.role === "user" ? "user" : "assistant", content));
      continue;
    }
    if (!Array.isArray(content)) {
      continue;
    }
    for (const part of content) {
      if (part.type === "text" && part.text) {
        flush();
        append(pane, bubble(message.role === "user" ? "user" : "assistant", part.text));
      } else if (part.type === "tool_use" && part.name) {
        steps.push(`${part.name} ${JSON.stringify(part.input ?? {})}`);
      } else if (part.type === "tool_result") {
        const detail = typeof part.content === "string" ? part.content : JSON.stringify(part.content ?? "");
        steps.push(detail);
      }
    }
  }
  flush();
}

function hasText(pane: Pane, kind: string, text: string): boolean {
  return [...pane.transcript.querySelectorAll(`.bubble.${kind}`)].some((node) => node.textContent === text);
}

async function loadTranscript(pane: Pane): Promise<void> {
  try {
    const chat = await invoke<ChatFile>("agent_transcript", { sessionId: pane.tab.sessionId });
    if (
      chat.provider === "anthropic" ||
      chat.provider === "openai" ||
      chat.provider === "deepseek" ||
      chat.provider === "auto"
    ) {
      pane.provider.value = chat.provider;
    }
    if (pane.transcript.childElementCount === 0) {
      paintStored(pane, chat.messages ?? []);
    }
  } catch (error) {
    logError("agent", error);
  } finally {
    pane.loaded = true;
    const waiting = pane.pending.splice(0);
    for (const event of waiting) {
      showEvent(pane, event);
    }
  }
}

async function paintHistory(): Promise<void> {
  const host = document.querySelector("#agent-history") as HTMLElement | null;
  if (!host || host.hidden) {
    return;
  }
  try {
    const chats = await invoke<ChatSummary[]>("agent_history");
    host.replaceChildren();
    if (chats.length === 0) {
      const empty = document.createElement("p");
      empty.textContent = "No chats yet.";
      host.append(empty);
      return;
    }
    for (const chat of chats) {
      const button = document.createElement("button");
      button.type = "button";
      const title = document.createElement("span");
      title.textContent = chat.title || "New chat";
      const meta = document.createElement("small");
      meta.textContent = providerLabel(chat.provider);
      button.title = chat.model ? `${providerLabel(chat.provider)} · ${chat.model}` : providerLabel(chat.provider);
      button.append(title, meta);
      button.addEventListener("click", () => {
        void invoke("workspace_open_chat", { sessionId: chat.sessionId }).catch((error: unknown) => {
          logError("agent", error);
        });
      });
      host.append(button);
    }
  } catch (error) {
    logError("agent", error);
  }
}

export function applyAgentEvent(event: AgentEvent): void {
  const pane = paneFor(event);
  if (!pane) {
    return;
  }
  if (!pane.loaded) {
    pane.pending.push(event);
    return;
  }
  showEvent(pane, event);
}

function gateButton(label: string, primary = false): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = label;
  if (primary) {
    button.className = "primary";
  }
  return button;
}

function showEvent(pane: Pane, event: AgentEvent): void {
  if (event.type === "assistant.text" && event.text) {
    if (hasText(pane, "assistant", event.text)) {
      return;
    }
    closeThinking(pane);
    append(pane, bubble("assistant", event.text));
  }
  if (event.type === "assistant.tool_call" && event.name) {
    addThinkingStep(pane, `${event.name} ${JSON.stringify(event.arguments ?? {})}`);
  }
  if (event.type === "tool.result" && event.name) {
    addThinkingStep(pane, `${event.name} ${event.ok === false ? "error" : "ok"}`);
  }
  if (event.type === "gate.ask" && event.callId && event.name) {
    pane.gate.hidden = false;
    pane.gate.replaceChildren();
    const text = document.createElement("p");
    text.textContent = `Allow ${event.name}?`;
    const args = document.createElement("pre");
    args.textContent = JSON.stringify(event.arguments ?? {}, null, 2);
    const actions = document.createElement("div");
    actions.className = "gate-actions";
    const allow = gateButton("Allow", true);
    const deny = gateButton("Deny");
    const similar = gateButton("Allow similar");
    const all = gateButton("Allow all tools");
    const callId = event.callId;
    const answer = (allowCall: boolean, scope: string) => {
      pane.gate.hidden = true;
      void invoke("gate_reply", { callId, allow: allowCall, scope });
    };
    allow.addEventListener("click", () => answer(true, "once"));
    deny.addEventListener("click", () => answer(false, "once"));
    similar.addEventListener("click", () => answer(true, "tool"));
    all.addEventListener("click", () => answer(true, "all"));
    actions.append(allow, deny, similar, all);
    pane.gate.append(text, args, actions);
  }
  if (event.type === "session.error" && event.text) {
    append(pane, bubble("assistant", event.text));
  }
  if (event.type === "session.result" && (event.text === "Stopped." || event.text === "Stopped after too many tool calls.")) {
    append(pane, bubble("assistant", event.text));
  }
  if (event.type === "session.result" || event.type === "session.error") {
    closeThinking(pane);
    setRunning(pane, false);
    void paintHistory();
  }
}

export function bootAgentUi(): void {
  const history = document.querySelector("#agent-history") as HTMLElement | null;
  const toggle = document.querySelector("#agent-history-toggle") as HTMLButtonElement | null;
  toggle?.addEventListener("click", () => {
    if (!history) {
      return;
    }
    history.hidden = !history.hidden;
    toggle.setAttribute("aria-expanded", history.hidden ? "false" : "true");
    void paintHistory();
  });
  void listen<AgentEvent>("agent-event", (event) => {
    applyAgentEvent(event.payload);
  }).catch((error: unknown) => {
    logError("agent", error);
  });
}

export function newAgentTab(): void {
  void invoke("workspace_new_agent").catch((error: unknown) => {
    logError("agent", error);
  });
}
