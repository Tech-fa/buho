import assert from "node:assert/strict";
import test from "node:test";
import { observationText } from "./observation.mjs";

test("a matched procedure is returned whole", () => {
  const procedure = "Follow only the procedure below.\n\n" + "x".repeat(12000);
  const result = {
    content: [{ type: "text", text: procedure }],
    isError: false,
  };
  const text = observationText(true, result, "", "fourss.task_guide");
  assert.equal(text, procedure);
  assert.equal(text.includes("[truncated]"), false);
});

test("other tool results stay capped", () => {
  const result = { content: [{ type: "text", text: "x".repeat(9000) }] };
  const text = observationText(true, result, "", "fourss.describe_page");
  assert.equal(text.includes("[truncated]"), true);
  assert.ok(text.length < 9000);
});
