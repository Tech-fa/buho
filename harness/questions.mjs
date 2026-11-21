const QUESTION_FENCE = /```[ \t]*buho-questions[^\n]*\n([\s\S]*?)```/g;
const ANSWER_FENCE = /^```[ \t]*buho-answers[^\n]*\n([\s\S]*?)```$/;
const ID_PATTERN = /^[A-Za-z][A-Za-z0-9_-]{0,40}$/;
const MAX_QUESTIONS = 8;
const MAX_OPTIONS = 12;
const MAX_TEXT = 2000;

export function askInstructions() {
  return [
    "When you need information from the user, stop and ask with the block below. Do not ask in a normal sentence. Do not guess a missing fact. Do not call a tool in that reply.",
    "Write a short explanation before the block, and put every question inside the block. Do not also list the questions as sentences or bullets. The app shows each prompt in the chat and lets the user answer with a checklist or an input box.",
    "```buho-questions",
    '{"questions":[{"id":"status","prompt":"Which status?","kind":"checklist","options":["Draft","Planned"],"multiple":false},{"id":"title","prompt":"What is the title?","kind":"input"}]}',
    "```",
    'kind is "checklist" or "input". A yes or no question is a checklist with those two options.',
    "A checklist needs at least two options. multiple false means the user picks one. multiple true means the user may pick several.",
    "An input is one text box and has no options.",
    "id starts with a letter and uses only letters, digits, hyphens, and underscores. prompt is the question the user reads. Do not copy the example unless those are the real questions.",
    "The user replies with one buho-answers block. Each answer uses the question id. Continue from those answers.",
  ].join("\n");
}

export function splitAsk(text) {
  const source = String(text || "");
  const questions = [];
  const removals = [];
  for (const match of source.matchAll(QUESTION_FENCE)) {
    const parsed = parseQuestions(match[1], questions);
    if (parsed.length === 0) continue;
    questions.push(...parsed);
    removals.push(match[0]);
  }
  let prose = source;
  for (const block of removals) {
    prose = prose.replace(block, "");
  }
  return {
    prose: prose.replace(/\n{3,}/g, "\n\n").trim(),
    questions: questions.slice(0, MAX_QUESTIONS),
  };
}

export function chatText(text) {
  const split = splitAsk(text);
  if (split.questions.length === 0) {
    return { text: String(text || ""), questions: [] };
  }
  const body = [split.prose, ...split.questions.map((question) => question.prompt)].filter(Boolean).join("\n\n");
  return { text: body, questions: split.questions };
}

export function answerMessage(questions, values) {
  const answers = [];
  for (const question of questions || []) {
    const value = answerValue(question, values?.[question.id]);
    if (value === null) continue;
    answers.push({ id: question.id, prompt: question.prompt, value });
  }
  return "```buho-answers\n" + JSON.stringify({ answers }) + "\n```";
}

export function readAnswers(text) {
  const match = String(text || "").trim().match(ANSWER_FENCE);
  if (!match) return null;
  let payload;
  try {
    payload = JSON.parse(match[1].trim());
  } catch {
    return null;
  }
  const list = Array.isArray(payload?.answers) ? payload.answers : null;
  if (!list) return null;
  const answers = [];
  for (const item of list) {
    const id = String(item?.id || "").trim();
    const prompt = String(item?.prompt || "").trim();
    if (!id || !prompt) continue;
    const value = item?.value;
    if (Array.isArray(value)) {
      const selected = value.map((entry) => String(entry || "").trim()).filter(Boolean);
      if (selected.length === 0) continue;
      answers.push({ id, prompt, value: selected });
      continue;
    }
    const textValue = String(value || "").trim();
    if (!textValue) continue;
    answers.push({ id, prompt, value: textValue });
  }
  return answers.length ? answers : null;
}

export function answerChat(text) {
  const answers = readAnswers(text);
  if (!answers) return null;
  const lines = answers.map((answer) => (Array.isArray(answer.value) ? answer.value.join(", ") : answer.value));
  return lines.length ? lines.join("\n") : null;
}

function parseQuestions(raw, existing) {
  let payload;
  try {
    payload = JSON.parse(String(raw || "").trim());
  } catch {
    return [];
  }
  const list = Array.isArray(payload) ? payload : payload?.questions;
  if (!Array.isArray(list)) return [];
  const questions = [];
  const used = new Set((existing || []).map((question) => question.id));
  for (const item of list) {
    if (existing.length + questions.length >= MAX_QUESTIONS) break;
    const prompt = clean(item?.prompt, 240);
    if (!prompt) continue;
    const id = questionId(item?.id, existing.length + questions.length, used);
    const kind = questionKind(item?.kind);
    if (kind === "input") {
      questions.push({ id, prompt, kind, options: [], multiple: false });
      continue;
    }
    if (kind !== "checklist") continue;
    const options = questionOptions(item?.options);
    if (options.length < 2) continue;
    questions.push({
      id,
      prompt,
      kind,
      options,
      multiple: item?.multiple === true,
    });
  }
  return questions;
}

function questionId(raw, index, used) {
  let id = String(raw || "").trim();
  if (!ID_PATTERN.test(id) || used.has(id)) {
    id = `q${index + 1}`;
    let extra = 2;
    while (used.has(id)) {
      id = `q${index + 1}_${extra}`;
      extra += 1;
    }
  }
  used.add(id);
  return id;
}

function questionKind(kind) {
  if (kind === "input" || kind === "text") return "input";
  if (kind === "checklist" || kind === "choice" || kind === "select") return "checklist";
  return "";
}

function questionOptions(options) {
  if (!Array.isArray(options)) return [];
  const seen = new Set();
  const cleaned = [];
  for (const option of options) {
    const text = clean(option, 120);
    if (!text || seen.has(text)) continue;
    seen.add(text);
    cleaned.push(text);
    if (cleaned.length === MAX_OPTIONS) break;
  }
  return cleaned;
}

function answerValue(question, raw) {
  if (!question || (question.kind !== "input" && question.kind !== "checklist")) return null;
  if (question.kind === "input") {
    const text = clean(raw, MAX_TEXT);
    return text || null;
  }
  const selected = (Array.isArray(raw) ? raw : [raw]).map((entry) => clean(entry, 120)).filter(Boolean);
  const allowed = selected.filter((entry) => question.options.includes(entry));
  if (allowed.length === 0) return null;
  if (question.multiple) return allowed;
  return allowed[0];
}

function clean(value, max) {
  return String(value ?? "").trim().slice(0, max);
}
