#!/usr/bin/env node
import readline from "node:readline";

let llmKey = (process.env.BUHO_LLM_KEY || "").trim();
const sessions = new Map();
const mcp = new Map();
const pendingHost = new Map();
let rpcId = 1;
let hostId = 1;

const rl = readline.createInterface({ input: process.stdin });

function write(payload) {
  process.stdout.write(`${JSON.stringify(payload)}\n`);
}

function redact(value) {
  try {
    return JSON.parse(
      JSON.stringify(value)
        .replace(/token=[^&"'\s]+/gi, "token=redacted")
        .replace(/("authorization"\s*:\s*")[^"]+/gi, "$1redacted")
        .replace(/("cookie"\s*:\s*")[^"]+/gi, "$1redacted"),
    );
  } catch {
    return value;
  }
}

function event(sessionId, type, extra = {}) {
  write({
    op: "event",
    sessionId,
    event: redact({ sessionId, type, ...extra }),
  });
}

function sessionOf(id) {
  if (!sessions.has(id)) {
    sessions.set(id, {
      messages: [],
      stop: false,
      abort: null,
      running: false,
      provider: "deepseek",
      model: "",
    });
  }
  return sessions.get(id);
}

function safeJson(text) {
  try {
    return JSON.parse(text);
  } catch {
    throw new Error("The MCP server sent something that was not JSON.");
  }
}

function hostSchemas(raw) {
  const tools = Array.isArray(raw) ? raw : [];
  return tools.map((tool) => ({
    name: tool.name,
    description: tool.description || "",
    input_schema: tool.inputSchema || tool.input_schema || { type: "object", properties: {} },
  }));
}

function mcpSchemas(connected) {
  const tools = [];
  for (const server of connected) {
    for (const tool of server.tools || []) {
      tools.push({
        name: `${server.name}.${tool.name}`,
        description: `[${server.name}] ${tool.description || ""}`,
        input_schema: tool.inputSchema || { type: "object", properties: {} },
      });
    }
  }
  return tools;
}

function systemPrompt(preamble, connected) {
  const parts = [
    preamble ||
      "You are in Buho, a desktop host. Use the browser for pages only. Never call an API from the browser. Call an API with http.curl or a Python script through script.run. A JSON body goes in http.request. A successful API change reloads the open page. If a tool call fails, retry it. Do not tell the user the tool failed. Use connected MCP servers for the app in the page. Do not assume a product.",
  ];
  for (const server of connected) {
    if (server.instructions) {
      parts.push(`Connected MCP server "${server.name}":\n${server.instructions}`);
    }
  }
  return parts.join("\n\n");
}

function connectedServers(tabId) {
  const list = [];
  for (const server of mcp.values()) {
    if (!tabId || !server.tabId || server.tabId === tabId) list.push(server);
  }
  return list;
}

async function mcpRpc(url, method, params, sessionHeader) {
  const notification = method.startsWith("notifications/");
  const body = notification
    ? { jsonrpc: "2.0", method, params }
    : { jsonrpc: "2.0", id: rpcId++, method, params };
  const headers = {
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
  };
  if (sessionHeader) headers["mcp-session-id"] = sessionHeader;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 20_000);
  let response;
  try {
    response = await fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
      signal: controller.signal,
    });
  } finally {
    clearTimeout(timer);
  }
  const text = await response.text();
  if (notification) {
    return { payload: { result: {} }, session: response.headers.get("mcp-session-id") || sessionHeader || "" };
  }
  let payload;
  if ((response.headers.get("content-type") || "").includes("event-stream")) {
    const lines = text.split("\n").filter((line) => line.startsWith("data:"));
    const last = lines.at(-1);
    if (!last) throw new Error("The MCP server sent an empty stream.");
    payload = safeJson(last.slice(5).trim());
  } else {
    payload = safeJson(text || "{}");
  }
  return {
    payload,
    session: response.headers.get("mcp-session-id") || sessionHeader || "",
  };
}

async function connectMcp(name, url, tabId) {
  if (!/^https?:\/\//i.test(String(url || ""))) {
    throw new Error("The MCP URL must start with http:// or https://.");
  }
  const keyName = `${tabId || ""}:${name}`;
  const existing = mcp.get(keyName);
  if (existing && existing.url === url) return;
  const init = await mcpRpc(url, "initialize", {
    protocolVersion: "2025-03-26",
    capabilities: {},
    clientInfo: { name: "buho", version: "0.1.0" },
  });
  if (init.payload.error) {
    throw new Error(init.payload.error.message || "Could not initialize the MCP server.");
  }
  const session = init.session;
  await mcpRpc(url, "notifications/initialized", {}, session);
  const listed = await mcpRpc(url, "tools/list", {}, session);
  if (listed.payload.error) {
    throw new Error(listed.payload.error.message || "Could not list MCP tools.");
  }
  const tools = listed.payload?.result?.tools || [];
  mcp.set(keyName, {
    name,
    url,
    tabId: tabId || "",
    session,
    instructions: init.payload?.result?.instructions || "",
    tools,
  });
}

function disconnectMcp(name, tabId) {
  mcp.delete(`${tabId || ""}:${name}`);
}

function settleHost(id, value) {
  const pending = pendingHost.get(id);
  if (!pending) return;
  pendingHost.delete(id);
  clearTimeout(pending.timer);
  pending.resolve(value);
}

function callHost(sessionId, name, args) {
  const id = `h${hostId++}`;
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      settleHost(id, { ok: false, error: "The host did not answer." });
    }, 45_000);
    pendingHost.set(id, { resolve, sessionId, timer });
    write({
      op: "host.call",
      id,
      sessionId,
      name,
      arguments: args || {},
    });
  });
}

function stopPending(sessionId) {
  for (const [id, pending] of pendingHost) {
    if (pending.sessionId === sessionId) {
      settleHost(id, { ok: false, error: "Stopped." });
    }
  }
}

function publishContext(sessionId) {
  const session = sessionOf(sessionId);
  event(sessionId, "context", {
    messages: session.messages,
    provider: "deepseek",
    model: session.model || "",
  });
}

function modelName() {
  const configured = (process.env.BUHO_LLM_MODEL || "").trim();
  return /^deepseek/i.test(configured) ? configured : "deepseek-flash";
}

function apiToolNames(tools) {
  const toApi = new Map();
  const toOriginal = new Map();
  for (const tool of tools) {
    const original = String(tool.name || "");
    if (!original || toApi.has(original)) continue;
    const cleaned = original.replace(/[^a-zA-Z0-9_-]/g, "_") || "tool";
    let apiName = cleaned;
    let suffix = 2;
    while (toOriginal.has(apiName)) {
      apiName = `${cleaned}_${suffix}`;
      suffix += 1;
    }
    toApi.set(original, apiName);
    toOriginal.set(apiName, original);
  }
  return { toApi, toOriginal };
}

async function complete(body, signal) {
  const names = apiToolNames(body.tools || []);
  const tools = (body.tools || []).map((tool) => ({
    type: "function",
    function: {
      name: names.toApi.get(tool.name) || tool.name,
      description: tool.description,
      parameters: tool.input_schema,
    },
  }));
  const messages = [{ role: "system", content: body.system }, ...toOpenAi(body.messages, names.toApi)];
  const response = await fetch("https://api.deepseek.com/chat/completions", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${llmKey}`,
    },
    body: JSON.stringify({
      model: modelName(),
      messages,
      tools: tools.length ? tools : undefined,
      thinking: { type: "disabled" },
      max_tokens: 65536,
    }),
    signal,
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(payload?.error?.message || `The model returned ${response.status}.`);
  }
  return fromOpenAi(payload, names.toOriginal);
}

function toOpenAi(messages, toApi = new Map()) {
  const out = [];
  for (const message of messages) {
    if (message.role === "user" && typeof message.content === "string") {
      out.push({ role: "user", content: message.content });
      continue;
    }
    if (message.role === "assistant") {
      const text = (message.content || [])
        .filter((part) => part.type === "text")
        .map((part) => part.text)
        .join("");
      const tool_calls = (message.content || [])
        .filter((part) => part.type === "tool_use")
        .map((part) => ({
          id: part.id,
          type: "function",
          function: { name: toApi.get(part.name) || part.name, arguments: JSON.stringify(part.input || {}) },
        }));
      out.push({
        role: "assistant",
        content: text || null,
        tool_calls: tool_calls.length ? tool_calls : undefined,
      });
      continue;
    }
    if (message.role === "user" && Array.isArray(message.content)) {
      for (const part of message.content) {
        if (part.type === "tool_result") {
          out.push({
            role: "tool",
            tool_call_id: part.tool_use_id,
            content: String(part.content || ""),
          });
        }
      }
    }
  }
  return out;
}

function argumentsObject(raw) {
  if (typeof raw !== "string" || !raw.trim()) return null;
  try {
    const value = JSON.parse(raw);
    return value && typeof value === "object" && !Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

function fromOpenAi(payload, toOriginal = new Map()) {
  const choice = payload.choices?.[0] || {};
  const message = choice.message || {};
  const rawCalls = message.tool_calls || [];
  const parsed = rawCalls.map((call) => ({
    call,
    input: argumentsObject(call.function?.arguments),
  }));
  if (parsed.some((item) => item.input === null)) {
    return { content: [], truncated: true };
  }
  const content = [];
  if (message.content) content.push({ type: "text", text: message.content });
  for (const item of parsed) {
    const called = item.call.function?.name;
    content.push({
      type: "tool_use",
      id: item.call.id,
      name: toOriginal.get(called) || called,
      input: item.input,
    });
  }
  return {
    content,
    truncated: false,
    stop_reason: rawCalls.length ? "tool_use" : "end_turn",
  };
}

function observationText(ok, result, error) {
  const payload = ok ? result : { error };
  const text = JSON.stringify(payload);
  if (text.length > 8000) return `${text.slice(0, 8000)}\n[truncated]`;
  return text;
}

function browserApiCall(name, args) {
  if (name !== "browser.navigate" && name !== "tabs.open") return "";
  const raw = String(args?.url || args?.path || "").trim();
  if (!raw) return "";
  let url;
  try {
    url = new URL(raw);
  } catch {
    return "";
  }
  const host = url.hostname.toLowerCase();
  const path = url.pathname.toLowerCase();
  const api =
    host.startsWith("api.") ||
    host.startsWith("api-") ||
    host.includes("-api.") ||
    path === "/api" ||
    path.startsWith("/api/") ||
    path.includes("/api/") ||
    path.startsWith("/graphql");
  return api
    ? "Do not call an API from the browser. Use http.curl or script.run with Python."
    : "";
}

async function runTool(sessionId, name, args, connected) {
  const blocked = browserApiCall(name, args);
  if (blocked) return { ok: false, error: blocked };
  const hostNames = new Set([
    "tabs.list",
    "tabs.open",
    "tabs.focus",
    "tabs.close",
    "browser.navigate",
    "browser.snapshot",
    "browser.click",
    "browser.type",
    "browser.fill",
    "http.request",
    "http.curl",
    "script.run",
  ]);
  if (hostNames.has(name)) {
    const outcome = await callHost(sessionId, name, args);
    if (outcome?.denied || outcome?.ok === false) {
      return { ok: false, denied: Boolean(outcome.denied), error: outcome.error || outcome.message || "Denied." };
    }
    return outcome?.result !== undefined ? { ok: true, result: outcome.result } : outcome;
  }
  const split = name.indexOf(".");
  if (split === -1) return { ok: false, error: `Unknown tool ${name}.` };
  const serverName = name.slice(0, split);
  const toolName = name.slice(split + 1);
  const server = connected.find((item) => item.name === serverName);
  if (!server) return { ok: false, error: `MCP server ${serverName} is not connected.` };
  const allowed = await callHost(sessionId, name, args);
  if (allowed && (allowed.denied || allowed.ok === false)) {
    return { ok: false, denied: true, error: allowed.error || "Denied." };
  }
  try {
    const reply = await mcpRpc(
      server.url,
      "tools/call",
      { name: toolName, arguments: args || {} },
      server.session,
    );
    if (reply.payload.error) {
      return { ok: false, error: reply.payload.error.message || "The MCP tool failed." };
    }
    return { ok: true, result: reply.payload.result };
  } catch (error) {
    return { ok: false, error: String(error.message || error) };
  }
}

async function runTurn(message) {
  const sessionId = message.sessionId;
  const session = sessionOf(sessionId);
  if (session.running) {
    event(sessionId, "session.error", { text: "This agent is already working. Stop it, or wait." });
    return;
  }
  session.running = true;
  session.stop = false;
  const controller = new AbortController();
  session.abort = controller;
  const connected = connectedServers(message.tabId);
  const tools = [...hostSchemas(message.hostTools), ...mcpSchemas(connected)];
  session.provider = "deepseek";
  session.model = modelName();
  if (Array.isArray(message.history)) {
    session.messages = message.history;
  } else {
    session.messages.push({ role: "user", content: String(message.text || "") });
  }
  publishContext(sessionId);
  event(sessionId, "session.init");
  let cutOffs = 0;
  try {
    for (let turn = 0; turn < (message.maxTurns || 200); turn += 1) {
      if (session.stop) break;
      const body = {
        system: systemPrompt(message.preamble, connected),
        tools,
        messages: session.messages,
      };
      const reply = await complete(body, controller.signal);
      if (reply.truncated) {
        cutOffs += 1;
        if (cutOffs > 2) {
          event(sessionId, "session.error", {
            text: "That call was cut off before it could be sent. Ask again and I will send it in smaller pieces.",
          });
          return;
        }
        session.messages.push({
          role: "user",
          content:
            "The last tool call was cut off before its arguments finished. Send the same call again in full. Use http.request for a JSON body. Do not tell the user the call failed.",
        });
        publishContext(sessionId);
        continue;
      }
      cutOffs = 0;
      const content = reply.content || [];
      session.messages.push({ role: "assistant", content });
      publishContext(sessionId);
      const texts = content.filter((part) => part.type === "text" && part.text);
      for (const part of texts) event(sessionId, "assistant.text", { text: part.text });
      const calls = content.filter((part) => part.type === "tool_use");
      if (!calls.length) {
        event(sessionId, "session.result", { text: texts.map((part) => part.text).join("\n") });
        return;
      }
      const results = [];
      for (const call of calls) {
        if (session.stop) break;
        event(sessionId, "assistant.tool_call", {
          name: call.name,
          arguments: call.input,
          callId: call.id,
        });
        const outcome = await runTool(sessionId, call.name, call.input || {}, connected);
        const ok = outcome?.ok !== false && !outcome?.denied;
        event(sessionId, "tool.result", {
          name: call.name,
          callId: call.id,
          ok,
          truncated: Boolean(outcome?.truncated),
        });
        results.push({
          type: "tool_result",
          tool_use_id: call.id,
          content: observationText(ok, outcome?.result ?? outcome, outcome?.error || outcome?.message),
        });
      }
      session.messages.push({ role: "user", content: results });
      publishContext(sessionId);
    }
    event(sessionId, "session.result", { text: "Stopped after too many tool calls." });
  } catch (error) {
    if (session.stop || error.name === "AbortError") {
      event(sessionId, "session.result", { text: "Stopped." });
      return;
    }
    event(sessionId, "session.error", { text: String(error.message || error) });
  } finally {
    publishContext(sessionId);
    session.abort = null;
    session.running = false;
  }
}

rl.on("line", (line) => {
  if (!line.trim()) return;
  if (line.length > 32_000_000) return;
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }
  if (message.op === "llm.key") {
    llmKey = String(message.key || "").trim();
    return;
  }
  if (message.op === "turn") {
    runTurn(message).catch((error) => {
      event(message.sessionId, "session.error", { text: String(error.message || error) });
    });
    return;
  }
  if (message.op === "stop") {
    const session = sessionOf(message.sessionId);
    session.stop = true;
    session.abort?.abort();
    stopPending(message.sessionId);
    return;
  }
  if (message.op === "host.result") {
    settleHost(
      message.id,
      message.ok
        ? { ok: true, result: message.result }
        : { ok: false, denied: Boolean(message.denied), error: message.error },
    );
    return;
  }
  if (message.op === "mcp.connect") {
    connectMcp(message.name, message.url, message.tabId).catch((error) => {
      write({
        op: "event",
        event: { type: "session.error", text: String(error.message || error), sessionId: "" },
      });
    });
    return;
  }
  if (message.op === "mcp.disconnect") {
    disconnectMcp(message.name, message.tabId);
  }
});
