// A matched procedure is longer than the usual tool-result cap.
// The audit program procedure alone is about 11,000 characters.
const PROCEDURE_LIMIT = 48000;
const TOOL_RESULT_LIMIT = 8000;

export function observationText(ok, result, error, toolName) {
  if (ok && String(toolName || "").endsWith(".task_guide")) {
    const parts = Array.isArray(result?.content) ? result.content : [];
    const text = parts
      .map((part) => (part && typeof part.text === "string" ? part.text : ""))
      .filter(Boolean)
      .join("\n");
    if (text) return cap(text, PROCEDURE_LIMIT);
  }
  const payload = ok ? result : { error };
  return cap(JSON.stringify(payload), TOOL_RESULT_LIMIT);
}

function cap(text, limit) {
  if (text.length > limit) return `${text.slice(0, limit)}\n[truncated]`;
  return text;
}
