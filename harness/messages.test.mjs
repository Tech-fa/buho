import assert from "node:assert/strict";
import test from "node:test";
import { toOpenAi } from "./messages.mjs";

test("an empty assistant reply is left out of the next request", () => {
  const messages = toOpenAi([
    { role: "user", content: "Continue." },
    { role: "assistant", content: [] },
    { role: "user", content: "Try again." },
  ]);
  assert.deepEqual(messages, [
    { role: "user", content: "Continue." },
    { role: "user", content: "Try again." },
  ]);
});

test("a tool call keeps an empty answer and its reasoning", () => {
  const messages = toOpenAi([
    {
      role: "assistant",
      reasoning_content: "",
      content: [{ type: "tool_use", id: "call-1", name: "tabs.list", input: {} }],
    },
  ]);
  assert.deepEqual(messages, [
    {
      role: "assistant",
      content: "",
      reasoning_content: "",
      tool_calls: [
        {
          id: "call-1",
          type: "function",
          function: { name: "tabs.list", arguments: "{}" },
        },
      ],
    },
  ]);
});
