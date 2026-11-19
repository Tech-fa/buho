export type BrowserTab = {
  kind: "browser";
  id: string;
  instanceId: string | null;
  url: string;
  title: string;
  webviewLabel: string;
  loading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
};

export type AgentTab = {
  kind: "agent";
  id: string;
  title: string;
  sessionId: string;
  boundTabIds: string[];
};

export type WorkspaceState = {
  instanceId: string;
  browserTabs: BrowserTab[];
  agentTabs: AgentTab[];
  activeBrowserId: string | null;
  activeAgentId: string | null;
  llmKeySet: boolean;
};

export type AgentEvent = {
  sessionId: string;
  type:
    | "session.init"
    | "assistant.text"
    | "assistant.tool_call"
    | "gate.ask"
    | "tool.result"
    | "session.result"
    | "session.error";
  text?: string;
  name?: string;
  arguments?: unknown;
  callId?: string;
  ok?: boolean;
  truncated?: boolean;
};

export type McpConnection = {
  id: string;
  name: string;
  url: string;
  tabId: string | null;
  status: "connecting" | "connected" | "error";
  toolCount: number;
};
