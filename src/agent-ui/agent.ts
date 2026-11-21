import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { answerChat, answerMessage, chatText } from "../../harness/questions.mjs";
import type { AskQuestion } from "../../harness/questions.mjs";
import { logError } from "../log";
import type { AgentEvent, AgentTab } from "../contracts/events";

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
  files: string[];
  fileRow: HTMLElement;
  gate: HTMLElement;
  askForm: HTMLFormElement;
  ask: AskQuestion[];
  running: boolean;
  watch: number | null;
  loaded: boolean;
  pending: AgentEvent[];
};

const LLM_LABEL = "buho_llm";

const panes = new Map<string, Pane>();
let activeId = "";
let llmKeySet = false;
let llmKeyKnown = false;
let llmEditorOpen = false;

export function setLlmKeySet(value: boolean): void {
  llmKeySet = value;
  llmKeyKnown = true;
  paintLlmForm();
}

function paintLlmForm(): void {
  const form = document.querySelector("#llm-key") as HTMLFormElement | null;
  const toggle = document.querySelector("#llm-key-toggle") as HTMLButtonElement | null;
  const help = document.querySelector("#llm-key-help") as HTMLParagraphElement | null;
  const remove = document.querySelector("#llm-key-remove") as HTMLButtonElement | null;
  const open = llmEditorOpen || (llmKeyKnown && !llmKeySet);
  if (form) {
    form.hidden = !open;
  }
  if (toggle) {
    toggle.setAttribute("aria-expanded", open ? "true" : "false");
  }
  if (help) {
    help.textContent = llmKeySet
      ? "Paste a new buho_llm token. It replaces the one saved on this computer."
      : "Paste your buho_llm token. It stays on this computer.";
  }
  if (remove) {
    remove.hidden = !llmKeySet;
  }
}

function llmError(message: string): void {
  const error = document.querySelector("#llm-key-error") as HTMLParagraphElement | null;
  if (error) {
    error.textContent = message;
  }
}

function openLlmForm(message = ""): void {
  llmEditorOpen = true;
  llmError(message);
  paintLlmForm();
  const input = document.querySelector("#llm-key-input") as HTMLInputElement | null;
  input?.focus();
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
  const askSubmit = pane.askForm.querySelector("button.primary");
  if (askSubmit instanceof HTMLButtonElement) {
    askSubmit.disabled = running;
  }
  refreshSend(pane);
  if (!running) {
    focusAsk(pane);
  }
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
  const transcript = document.createElement("div");
  transcript.className = "transcript";
  const gate = document.createElement("div");
  gate.className = "gate-card";
  gate.hidden = true;
  const askForm = document.createElement("form");
  askForm.className = "ask-card";
  askForm.hidden = true;
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
  const send = document.createElement("button");
  send.type = "button";
  send.className = "composer-send";
  send.disabled = true;
  send.innerHTML =
    '<svg class="icon-send" viewBox="0 0 16 16" aria-hidden="true"><path d="M8 12.2V3.8M4.6 7.2 8 3.8l3.4 3.4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg><svg class="icon-stop" viewBox="0 0 16 16" aria-hidden="true"><rect x="5" y="5" width="6" height="6" rx="1" fill="currentColor"/></svg>';
  const bar = document.createElement("div");
  bar.className = "composer-bar";
  bar.append(attach, send);
  const row = document.createElement("div");
  row.className = "composer";
  row.append(fileRow, composer, bar);
  root.append(transcript, gate, askForm, row);
  host.append(root);
  const pane: Pane = {
    tab,
    root,
    transcript,
    composer,
    send,
    attach,
    files: [],
    fileRow,
    gate,
    askForm,
    ask: [],
    running: false,
    watch: null,
    loaded: false,
    pending: [],
  };
  refreshSend(pane);
  attach.addEventListener("click", () => void attachFiles(pane));
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
  askForm.addEventListener("submit", (event) => {
    event.preventDefault();
    const questions = pane.ask;
    if (pane.running || questions.length === 0) {
      return;
    }
    const values = readAsk(askForm, questions);
    const error = askForm.querySelector(".ask-error");
    if (!values) {
      if (error instanceof HTMLParagraphElement) {
        error.hidden = false;
        error.textContent = "Answer each question.";
      }
      return;
    }
    const text = answerMessage(questions, values);
    paintAsk(pane, []);
    void deliver(pane, text, answerChat(text) || text);
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
  paintAsk(pane, []);
  void deliver(pane, text, text);
}

async function deliver(pane: Pane, text: string, shown: string): Promise<void> {
  append(pane, bubble("user", shown));
  setRunning(pane, true);
  try {
    const outcome = await invoke<{ ok: boolean; message?: string; needsKey?: boolean }>("agent_send", {
      sessionId: pane.tab.sessionId,
      text,
    });
    if (outcome && outcome.ok === false) {
      if (outcome.needsKey) {
        openLlmForm(outcome.message || "Paste a buho_llm token to talk to the model.");
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

function paintStored(pane: Pane, messages: StoredMessage[]): void {
  const steps: string[] = [];
  let pendingQuestions: AskQuestion[] = [];
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
  const showUser = (text: string) => {
    pendingQuestions = [];
    const node = bubble("user", answerChat(text) || text);
    node.dataset.raw = text;
    append(pane, node);
  };
  for (const message of messages) {
    const content = message.content;
    if (typeof content === "string") {
      flush();
      if (message.role === "user") {
        showUser(content);
      } else {
        pendingQuestions = showAssistant(pane, content);
      }
      continue;
    }
    if (!Array.isArray(content)) {
      continue;
    }
    if (message.role === "user") {
      pendingQuestions = [];
    }
    for (const part of content) {
      if (part.type === "text" && part.text) {
        flush();
        if (message.role === "user") {
          showUser(part.text);
        } else {
          pendingQuestions = showAssistant(pane, part.text);
        }
      } else if (part.type === "tool_use" && part.name) {
        steps.push(`${part.name} ${JSON.stringify(part.input ?? {})}`);
      } else if (part.type === "tool_result") {
        const detail = typeof part.content === "string" ? part.content : JSON.stringify(part.content ?? "");
        steps.push(detail);
      }
    }
  }
  flush();
  paintAsk(pane, pendingQuestions);
}

async function loadTranscript(pane: Pane): Promise<void> {
  try {
    const chat = await invoke<ChatFile>("agent_transcript", { sessionId: pane.tab.sessionId });
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
      meta.textContent = LLM_LABEL;
      button.title = LLM_LABEL;
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

function hasRaw(pane: Pane, text: string): boolean {
  return [...pane.transcript.querySelectorAll<HTMLElement>("[data-raw]")].some((node) => node.dataset.raw === text);
}

function showAssistant(pane: Pane, text: string): AskQuestion[] {
  const shown = chatText(text);
  const node = bubble("assistant", shown.text);
  node.dataset.raw = text;
  append(pane, node);
  return shown.questions;
}

function paintAsk(pane: Pane, questions: AskQuestion[]): void {
  pane.ask = questions;
  const form = pane.askForm;
  form.replaceChildren();
  pane.composer.placeholder = questions.length === 0 ? "Ask the agent…" : "Type a reply…";
  if (questions.length === 0) {
    form.hidden = true;
    return;
  }
  form.hidden = false;
  const kicker = document.createElement("p");
  kicker.className = "ask-kicker";
  kicker.textContent = "Questions";
  form.append(kicker);
  for (const question of questions) {
    form.append(askItem(pane, question));
  }
  const error = document.createElement("p");
  error.className = "ask-error";
  error.setAttribute("role", "alert");
  error.hidden = true;
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.className = "primary";
  submit.textContent = "Send answers";
  submit.disabled = pane.running;
  form.append(error, submit);
  if (!pane.running) {
    focusAsk(pane);
  }
}

function askItem(pane: Pane, question: AskQuestion): HTMLElement {
  const item = document.createElement("div");
  item.className = "ask-item";
  const prompt = document.createElement("p");
  prompt.className = "ask-prompt";
  prompt.id = askDomId(pane, question.id);
  prompt.textContent = question.prompt;
  item.append(prompt);
  if (question.kind === "input") {
    const input = document.createElement("input");
    input.type = "text";
    input.name = question.id;
    input.autocomplete = "off";
    input.maxLength = 2000;
    input.setAttribute("aria-labelledby", prompt.id);
    item.append(input);
    return item;
  }
  const choices = document.createElement("div");
  choices.className = "choices";
  choices.setAttribute("role", question.multiple ? "group" : "radiogroup");
  choices.setAttribute("aria-labelledby", prompt.id);
  for (const option of question.options) {
    const label = document.createElement("label");
    label.className = "choice";
    const input = document.createElement("input");
    input.type = question.multiple ? "checkbox" : "radio";
    input.name = question.id;
    input.value = option;
    const text = document.createElement("span");
    text.textContent = option;
    label.append(input, text);
    choices.append(label);
  }
  item.append(choices);
  return item;
}

function askDomId(pane: Pane, questionId: string): string {
  return `ask-${pane.tab.id}-${questionId}`.replace(/[^A-Za-z0-9_-]/g, "-");
}

function readAsk(form: HTMLFormElement, questions: AskQuestion[]): Record<string, string | string[]> | null {
  const values: Record<string, string | string[]> = {};
  for (const question of questions) {
    if (question.kind === "input") {
      const field = form.querySelector(`input[type="text"][name="${CSS.escape(question.id)}"]`);
      const text = field instanceof HTMLInputElement ? field.value.trim() : "";
      if (!text) {
        return null;
      }
      values[question.id] = text;
      continue;
    }
    const selected = [...form.querySelectorAll(`input[name="${CSS.escape(question.id)}"]:checked`)].flatMap((node) =>
      node instanceof HTMLInputElement ? [node.value] : [],
    );
    const allowed = selected.filter((value) => question.options.includes(value));
    if (allowed.length === 0 || (!question.multiple && allowed.length !== 1)) {
      return null;
    }
    values[question.id] = question.multiple ? allowed : allowed[0];
  }
  return values;
}

function focusAsk(pane: Pane): void {
  if (pane.root.hidden || pane.running || pane.askForm.hidden) {
    return;
  }
  if (document.activeElement === pane.composer) {
    return;
  }
  const first = pane.askForm.querySelector("input");
  if (first instanceof HTMLElement) {
    first.focus();
  }
}

function showEvent(pane: Pane, event: AgentEvent): void {
  if (event.type === "assistant.text" && event.text) {
    if (!hasRaw(pane, event.text)) {
      closeThinking(pane);
      paintAsk(pane, showAssistant(pane, event.text));
    }
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
    const body = document.createElement("div");
    body.className = "gate-body";
    body.append(text, args);
    pane.gate.append(body, actions);
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
  const keyToggle = document.querySelector("#llm-key-toggle") as HTMLButtonElement | null;
  const keyForm = document.querySelector("#llm-key") as HTMLFormElement | null;
  const keyInput = document.querySelector("#llm-key-input") as HTMLInputElement | null;
  const keyRemove = document.querySelector("#llm-key-remove") as HTMLButtonElement | null;
  keyToggle?.addEventListener("click", () => {
    if (!llmKeySet) {
      openLlmForm();
      return;
    }
    llmEditorOpen = !llmEditorOpen;
    if (llmEditorOpen) {
      llmError("");
    }
    paintLlmForm();
    if (llmEditorOpen) {
      keyInput?.focus();
    }
  });
  keyForm?.addEventListener("submit", (event) => {
    event.preventDefault();
    const token = keyInput?.value.trim() ?? "";
    if (!token) {
      llmError("Paste the buho_llm token.");
      return;
    }
    llmError("");
    void invoke("save_llm_key", { key: token })
      .then(() => {
        if (keyInput) {
          keyInput.value = "";
        }
        llmEditorOpen = false;
        setLlmKeySet(true);
      })
      .catch((error: unknown) => {
        logError("buho_llm", error);
        llmError(String(error));
      });
  });
  keyRemove?.addEventListener("click", () => {
    llmError("");
    void invoke("clear_llm_key")
      .then(() => {
        if (keyInput) {
          keyInput.value = "";
        }
        llmEditorOpen = true;
        setLlmKeySet(false);
      })
      .catch((error: unknown) => {
        logError("buho_llm", error);
        llmError(String(error));
      });
  });
  paintLlmForm();
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
