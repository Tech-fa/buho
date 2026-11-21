export type AskQuestion = {
  id: string;
  prompt: string;
  kind: "checklist" | "input";
  options: string[];
  multiple: boolean;
};

export function askInstructions(): string;

export function splitAsk(text: string): { prose: string; questions: AskQuestion[] };

export function chatText(text: string): { text: string; questions: AskQuestion[] };

export function answerMessage(
  questions: AskQuestion[],
  values: Record<string, string | string[]>,
): string;

export function readAnswers(
  text: string,
): Array<{ id: string; prompt: string; value: string | string[] }> | null;

export function answerChat(text: string): string | null;
