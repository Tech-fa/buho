import assert from "node:assert/strict";
import test from "node:test";
import { answerChat, answerMessage, askInstructions, chatText, readAnswers, splitAsk } from "./questions.mjs";

const sample = [
  "I need two details before I continue.",
  "```buho-questions",
  JSON.stringify({
    questions: [
      {
        id: "status",
        prompt: "Which status should it start in?",
        kind: "checklist",
        options: ["Draft", "Planned"],
        multiple: false,
      },
      { id: "title", prompt: "What is the title?", kind: "input" },
    ],
  }),
  "```",
].join("\n");

test("a question block becomes chat text and structured questions", () => {
  const shown = chatText(sample);
  assert.equal(
    shown.text,
    "I need two details before I continue.\n\nWhich status should it start in?\n\nWhat is the title?",
  );
  assert.equal(shown.questions.length, 2);
  assert.equal(shown.questions[0].kind, "checklist");
  assert.equal(shown.questions[0].multiple, false);
  assert.deepEqual(shown.questions[0].options, ["Draft", "Planned"]);
  assert.equal(shown.questions[1].kind, "input");
  assert.equal(shown.text.includes("buho-questions"), false);
});

test("a checklist can allow several answers", () => {
  const shown = chatText(
    '```buho-questions\n{"questions":[{"id":"parts","prompt":"Which parts?","kind":"checklist","options":["Notes","Files"],"multiple":true}]}\n```',
  );
  assert.equal(shown.questions[0].multiple, true);
});

test("a broken block stays in the chat as text", () => {
  const text = "```buho-questions\n{not json}\n```";
  assert.deepEqual(splitAsk(text).questions, []);
  assert.equal(chatText(text).text, text);
});

test("a checklist with one option is left as text", () => {
  const text = '```buho-questions\n{"questions":[{"id":"only","prompt":"Pick","kind":"checklist","options":["Only"]}]}\n```';
  assert.equal(splitAsk(text).questions.length, 0);
});

test("answers round trip into the chat without the fence", () => {
  const questions = chatText(sample).questions;
  const sent = answerMessage(questions, { status: "Planned", title: "Q1 safety" });
  assert.equal(sent.includes("buho-answers"), true);
  assert.deepEqual(readAnswers(sent), [
    { id: "status", prompt: "Which status should it start in?", value: "Planned" },
    { id: "title", prompt: "What is the title?", value: "Q1 safety" },
  ]);
  assert.equal(answerChat(sent), "Planned\nQ1 safety");
});

test("several checklist answers stay a list", () => {
  const questions = [
    { id: "parts", prompt: "Which parts?", kind: "checklist", options: ["Notes", "Files"], multiple: true },
  ];
  const sent = answerMessage(questions, { parts: ["Files", "Notes"] });
  assert.equal(answerChat(sent), "Files, Notes");
});

test("the model is told to use the question fence", () => {
  const instructions = askInstructions();
  assert.equal(instructions.includes("```buho-questions"), true);
  assert.equal(instructions.includes('"kind":"checklist"'), true);
  assert.equal(instructions.includes('"kind":"input"'), true);
  assert.equal(instructions.includes("buho-answers"), true);
});
