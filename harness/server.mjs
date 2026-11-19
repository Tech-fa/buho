#!/usr/bin/env node
import readline from "node:readline";

const key = (process.env.BUHO_LLM_KEY || "").trim();
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
      provider: "auto",
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
      "You are in Buho, a desktop host. Use host tools for tabs and HTTP. Use connected MCP servers for the app in the page. Do not assume a product.",
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
    provider: session.provider || "auto",
    model: session.model || "",
  });
}

async function anthropic(body, signal) {
  const response = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      "x-api-key": key,
      "anthropic-version": "2023-06-01",
    },
    body: JSON.stringify(body),
    signal,
  });
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) {
    throw new Error(payload?.error?.message || `The model returned ${response.status}.`);
  }
  return payload;
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

async function openai(body, signal, model, options = {}) {
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
  const response = await fetch(options.url || "https://api.openai.com/v1/chat/completions", {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${key}`,
    },
    body: JSON.stringify({
      model: model || "gpt-4o",
      messages,
      tools: tools.length ? tools : undefined,
      ...options.extra,
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

function fromOpenAi(payload, toOriginal = new Map()) {
  const message = payload.choices?.[0]?.message || {};
  const content = [];
  if (message.content) content.push({ type: "text", text: message.content });
  for (const call of message.tool_calls || []) {
    let input = {};
    try {
      input = JSON.parse(call.function?.arguments || "{}");
    } catch {
      input = {};
    }
    const called = call.function?.name;
    content.push({
      type: "tool_use",
      id: call.id,
      name: toOriginal.get(called) || called,
      input,
    });
  }
  return { content, stop_reason: message.tool_calls?.length ? "tool_use" : "end_turn" };
}

function usesAnthropic() {
  return key.startsWith("sk-ant-") || !key.startsWith("sk-");
}

function resolveProvider(choice) {
  if (choice === "anthropic" || choice === "openai" || choice === "deepseek") return choice;
  return usesAnthropic() ? "anthropic" : "openai";
}

function modelFor(provider) {
  const configured = process.env.BUHO_LLM_MODEL || "";
  if (provider === "anthropic") {
    return /^claude/i.test(configured) ? configured : "claude-sonnet-4-20250514";
  }
  if (provider === "deepseek") {
    return /^deepseek/i.test(configured) ? configured : "deepseek-flash";
  }
  return configured && !/^claude|^deepseek/i.test(configured) ? configured : "gpt-4o";
}

async function complete(body, signal, provider) {
  const choice = resolveProvider(provider);
  if (choice === "anthropic") {
    return anthropic(
      {
        model: modelFor(choice),
        max_tokens: 4096,
        system: body.system,
        tools: body.tools,
        messages: body.messages,
      },
      signal,
    );
  }
  if (choice === "deepseek") {
    return openai(body, signal, modelFor(choice), {
      url: "https://api.deepseek.com/chat/completions",
      extra: { thinking: { type: "disabled" } },
    });
  }
  return openai(body, signal, modelFor(choice));
}

function observationText(ok, result, error) {
  const payload = ok ? result : { error };
  const text = JSON.stringify(payload);
  if (text.length > 8000) return `${text.slice(0, 8000)}\n[truncated]`;
  return text;
}

async function runTool(sessionId, name, args, connected) {
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
  const choice = resolveProvider(message.provider);
  session.provider =
    message.provider === "anthropic" || message.provider === "openai" || message.provider === "deepseek"
      ? message.provider
      : "auto";
  session.model = modelFor(choice);
  if (Array.isArray(message.history)) {
    session.messages = message.history;
  } else {
    session.messages.push({ role: "user", content: String(message.text || "") });
  }
  publishContext(sessionId);
  event(sessionId, "session.init");
  try {
    for (let turn = 0; turn < (message.maxTurns || 200); turn += 1) {
      if (session.stop) break;
      const body = {
        system: systemPrompt(message.preamble, connected),
        tools,
        messages: session.messages,
      };
      const reply = await complete(body, controller.signal, message.provider);
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
