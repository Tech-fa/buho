export function toOpenAi(messages, toApi = new Map()) {
  const out = [];
  for (const message of messages) {
    if (message.role === "user" && typeof message.content === "string") {
      out.push({ role: "user", content: message.content });
      continue;
    }
    if (message.role === "assistant") {
      const assistant = assistantMessage(message, toApi);
      if (assistant) out.push(assistant);
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

function assistantMessage(message, toApi) {
  const parts = Array.isArray(message.content)
    ? message.content
    : typeof message.content === "string" && message.content
      ? [{ type: "text", text: message.content }]
      : [];
  const text = parts
    .filter((part) => part.type === "text" && part.text)
    .map((part) => part.text)
    .join("");
  const tool_calls = parts
    .filter((part) => part.type === "tool_use")
    .map((part) => ({
      id: part.id,
      type: "function",
      function: { name: toApi.get(part.name) || part.name, arguments: JSON.stringify(part.input || {}) },
    }));
  const reasoning = message.reasoning_content;
  const hasReasoning = typeof reasoning === "string";
  if (!text && tool_calls.length === 0 && !hasReasoning) return null;
  const assistant = { role: "assistant", content: text };
  if (hasReasoning) assistant.reasoning_content = reasoning;
  if (tool_calls.length) assistant.tool_calls = tool_calls;
  return assistant;
}
