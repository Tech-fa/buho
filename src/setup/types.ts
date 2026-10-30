export type FieldType = "string" | "url" | "number" | "boolean" | "text";

export type Field = {
  key: string;
  label: string;
  description: string;
  type: FieldType;
  required: boolean;
  placeholder: string;
  default?: unknown;
};

export type ShellState = {
  uiPath: string;
  uiSource: string;
  serverUrl: string;
  manifestFound: boolean;
  appName: string;
  appId: string;
  configFile: string;
  fields: Field[];
  values: Record<string, unknown>;
  missingRequired: string[];
  readyToOpen: boolean;
  status: string;
  dataDir: string;
  appUrl: string;
  mcpUrl: string;
};

export type SaveOutcome = {
  state: ShellState;
  fieldErrors: Record<string, string>;
  message: string;
};

export type Deployment = {
  hasDigitalOceanToken: boolean;
  hasResendApiKey: boolean;
  region: string;
  size: string;
  dropletId: number;
  dropletName: string;
  publicIp: string;
  fromEmail: string;
  adminEmail: string;
  adminPassword: string;
  status: string;
  running: boolean;
  appUrl: string;
  apiVersion: string;
  uiVersion: string;
};

export type ReleaseVersions = {
  apiVersion: string;
  uiVersion: string;
};

export type RegionChoice = {
  slug: string;
  name: string;
};

export const TOKEN_URL = "https://cloud.digitalocean.com/account/api/tokens";
export const RESEND_URL = "https://resend.com/api-keys";
export const DROPLET_SIZE = "s-2vcpu-4gb";
