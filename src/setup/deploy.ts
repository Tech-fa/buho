import { invoke } from "@tauri-apps/api/core";
import { confirm, open, save } from "@tauri-apps/plugin-dialog";
import { logError } from "../log";
import { DROPLET_SIZE, RESEND_URL, TOKEN_URL, type AppUpdate, type Deployment, type RegionChoice, type ReleaseVersions } from "./types";

const DEFAULT_FROM_EMAIL = "notifications@resend.dev";
const WORKING = "Working on making your app. This can take a while. Leave this window open.";
const PLACE_UNAVAILABLE =
  "That place is not available. Choose another place close to your team, then try again.";
const READY = "Your app is ready.";

const appName = document.querySelector("#app-name") as HTMLHeadingElement;
const summary = document.querySelector("#summary") as HTMLParagraphElement;
const tokenPanel = document.querySelector("#token-panel") as HTMLElement;
const tokenState = document.querySelector("#token-state") as HTMLParagraphElement;
const hostState = document.querySelector("#host-state") as HTMLParagraphElement;
const hostDigitalOcean = document.querySelector("#host-digitalocean") as HTMLInputElement;
const hostServer = document.querySelector("#host-server") as HTMLInputElement;
const serverPanel = document.querySelector("#server-panel") as HTMLElement;
const serverState = document.querySelector("#server-state") as HTMLParagraphElement;
const serverEditor = document.querySelector("#server-editor") as HTMLDivElement;
const serverIp = document.querySelector("#server-ip") as HTMLInputElement;
const serverUser = document.querySelector("#server-user") as HTMLInputElement;
const serverKey = document.querySelector("#server-key") as HTMLInputElement;
const serverError = document.querySelector("#server-error") as HTMLParagraphElement;
const editServer = document.querySelector("#edit-server") as HTMLButtonElement;
const removeServer = document.querySelector("#remove-server") as HTMLButtonElement;
const saveServer = document.querySelector("#save-server") as HTMLButtonElement;
const importKey = document.querySelector("#import-key") as HTMLButtonElement;
const resendState = document.querySelector("#resend-state") as HTMLParagraphElement;
const fromEmailState = document.querySelector("#from-email-state") as HTMLParagraphElement;
const fromEmailFollowup = document.querySelector("#from-email-followup") as HTMLParagraphElement;
const adminEmailState = document.querySelector("#admin-email-state") as HTMLParagraphElement;
const adminEmailFollowup = document.querySelector("#admin-email-followup") as HTMLParagraphElement;
const dropletState = document.querySelector("#droplet-state") as HTMLParagraphElement;
const tokenEditor = document.querySelector("#token-editor") as HTMLDivElement;
const resendEditor = document.querySelector("#resend-editor") as HTMLDivElement;
const fromEmailEditor = document.querySelector("#from-email-editor") as HTMLDivElement;
const adminEmailEditor = document.querySelector("#admin-email-editor") as HTMLDivElement;
const regionStep = document.querySelector("#region-step") as HTMLDivElement;
const placeFields = document.querySelector("#place-fields") as HTMLDivElement;
const serverDeployNote = document.querySelector("#server-deploy-note") as HTMLParagraphElement;
const regionWarning = document.querySelector("#region-warning") as HTMLParagraphElement;
const dropletActions = document.querySelector("#droplet-actions") as HTMLDivElement;
const finished = document.querySelector("#finished") as HTMLElement;
const finishedDetail = document.querySelector("#finished-detail") as HTMLParagraphElement;
const downloadNote = document.querySelector("#download-note") as HTMLParagraphElement;
const backupNote = document.querySelector("#backup-note") as HTMLParagraphElement;
const downloadBackup = document.querySelector("#download-backup") as HTMLButtonElement;
const downloadConfig = document.querySelector("#download-config") as HTMLButtonElement;
const doToken = document.querySelector("#do-token") as HTMLInputElement;
const resendKey = document.querySelector("#resend-key") as HTMLInputElement;
const fromEmail = document.querySelector("#from-email") as HTMLInputElement;
const adminEmail = document.querySelector("#admin-email") as HTMLInputElement;
const regionSelect = document.querySelector("#region") as HTMLSelectElement;
const editToken = document.querySelector("#edit-token") as HTMLButtonElement;
const editResend = document.querySelector("#edit-resend") as HTMLButtonElement;
const editFromEmail = document.querySelector("#edit-from-email") as HTMLButtonElement;
const saveFromEmail = document.querySelector("#save-from-email") as HTMLButtonElement;
const editAdminEmail = document.querySelector("#edit-admin-email") as HTMLButtonElement;
const saveAdminEmail = document.querySelector("#save-admin-email") as HTMLButtonElement;
const removeToken = document.querySelector("#remove-token") as HTMLButtonElement;
const removeResend = document.querySelector("#remove-resend") as HTMLButtonElement;
const deployButton = document.querySelector("#deploy") as HTMLButtonElement;
const redeployButton = document.querySelector("#redeploy") as HTMLButtonElement;
const deleteButton = document.querySelector("#delete-droplet") as HTMLButtonElement;
const deployStatus = document.querySelector("#deploy-status") as HTMLParagraphElement;
const deployErrorBanner = document.querySelector("#deploy-error") as HTMLDivElement;
const deployErrorText = document.querySelector("#deploy-error-text") as HTMLParagraphElement;
const deployErrorClose = document.querySelector("#deploy-error-close") as HTMLButtonElement;
const tokenError = document.querySelector("#token-error") as HTMLParagraphElement;
const resendError = document.querySelector("#resend-error") as HTMLParagraphElement;
const fromEmailError = document.querySelector("#from-email-error") as HTMLParagraphElement;
const adminEmailError = document.querySelector("#admin-email-error") as HTMLParagraphElement;
const downloadError = document.querySelector("#download-error") as HTMLParagraphElement;
const backupError = document.querySelector("#backup-error") as HTMLParagraphElement;
const domainPanel = document.querySelector("#domain-panel") as HTMLElement;
const domainStatus = document.querySelector("#domain-status") as HTMLParagraphElement;
const domainErrorBanner = document.querySelector("#domain-error") as HTMLDivElement;
const domainErrorText = document.querySelector("#domain-error-text") as HTMLParagraphElement;
const domainErrorClose = document.querySelector("#domain-error-close") as HTMLButtonElement;
const releaseNote = document.querySelector("#release-note") as HTMLParagraphElement;
const runningActions = document.querySelector("#running-actions") as HTMLDivElement;
const pageForm = document.querySelector("#page-form") as HTMLFormElement;
const pageUrl = document.querySelector("#page-url") as HTMLInputElement;
const pageError = document.querySelector("#page-error") as HTMLParagraphElement;
const domainInput = document.querySelector("#domain") as HTMLInputElement;
const domainInstructions = document.querySelector("#domain-instructions") as HTMLDivElement;
const domainWhere = document.querySelector("#domain-where") as HTMLParagraphElement;
const appNameCell = document.querySelector("#dns-app-name") as HTMLTableCellElement;
const apiNameCell = document.querySelector("#dns-api-name") as HTMLTableCellElement;
const appValueCell = document.querySelector("#app-value") as HTMLTableCellElement;
const apiValueCell = document.querySelector("#api-value") as HTMLTableCellElement;
const domainNameHint = document.querySelector("#domain-name-hint") as HTMLParagraphElement;
const httpsState = document.querySelector("#https-state") as HTMLParagraphElement;
const saveDomainButton = document.querySelector("#save-domain") as HTMLButtonElement;
const enableHttpsButton = document.querySelector("#enable-https") as HTMLButtonElement;
const buhoUpdate = document.querySelector("#buho-update") as HTMLDivElement;
const buhoUpdateText = document.querySelector("#buho-update-text") as HTMLParagraphElement;
const buhoUpdateDownload = document.querySelector("#buho-update-download") as HTMLButtonElement;

let deployment: Deployment | null = null;
let tokenEditorOpen = false;
let serverEditorOpen = false;
let chosenKeyPath = "";
let shownServerIp = "";
let shownServerUser = "";
let resendEditorOpen = false;
let fromEmailEditorOpen = false;
let adminEmailEditorOpen = false;
let actionLocked = false;
let shownStatus = "";
let availableRelease: ReleaseVersions | null = null;
let releaseError = "";
let releaseLoaded = false;
let sawRunning = false;
let shownAppUrl = "";
let shownDomain = "";
let appDownloadUrl = "";
let activeWork: "" | "deploy" | "domain" = "";
let workWasRunning = false;
const dismissedDeployError = { value: "" };
const dismissedDomainError = { value: "" };

function isDomainSecurityMessage(message: string): boolean {
  const lower = message.toLowerCase();
  return (
    lower.includes("secure the domain") ||
    lower.includes("securing your domain") ||
    lower.includes("domain name") ||
    lower.includes("point at")
  );
}

function showErrorBanner(
  root: HTMLElement,
  text: HTMLElement,
  dismissed: { value: string },
  message: string,
): void {
  if (!message || message === dismissed.value) {
    return;
  }
  dismissed.value = "";
  text.textContent = message;
  root.hidden = false;
}

function closeErrorBanner(root: HTMLElement, text: HTMLElement, dismissed: { value: string }): void {
  dismissed.value = text.textContent ?? "";
  root.hidden = true;
}

function showDeployError(message: string): void {
  showErrorBanner(deployErrorBanner, deployErrorText, dismissedDeployError, message);
}

function showDomainError(message: string): void {
  showErrorBanner(domainErrorBanner, domainErrorText, dismissedDomainError, message);
}

function plainMessage(message: string): string {
  const text = message.replace(/^Error:\s*/i, "");
  const lower = text.toLowerCase();
  if (lower.includes(PLACE_UNAVAILABLE.toLowerCase())) {
    return PLACE_UNAVAILABLE;
  }
  const placeUnavailable =
    lower.includes("invalid region") ||
    ((lower.includes("region") || lower.includes("size")) &&
      (lower.includes("not available") || lower.includes("unavailable")));
  if (placeUnavailable) {
    return PLACE_UNAVAILABLE;
  }
  if (lower.includes("not authorized") || lower.includes("returned 403")) {
    return "DigitalOcean did not accept this key. Create a new key and choose Full Access, as in the picture.";
  }
  if (lower.includes("point at")) {
    return text;
  }
  if (lower.includes("remote command failed") || lower.includes("failed to solve") || text.length > 240) {
    return "We could not finish making your app. Wait a moment, then try again.";
  }
  return text;
}

function showDomainRecords(saved: string, typed: string, ip: string): void {
  const current = typed.trim().toLowerCase().replace(/\.+$/, "");
  const show = saved.length > 0 && current === saved && ip.length > 0;
  domainInstructions.hidden = !show;
  if (!show) {
    return;
  }
  domainWhere.textContent = `Open the website where you bought ${saved}. Find DNS, or the place where records are listed, and add these two rows.`;
  appNameCell.textContent = "4ss-app";
  apiNameCell.textContent = "4ss-app-api";
  appValueCell.textContent = ip;
  apiValueCell.textContent = ip;
  domainNameHint.textContent = `In the Name box, type only what the table shows. The website already adds .${saved}. The Value box is the address.`;
}

function askForAnotherPlace(rejected?: string): void {
  if (rejected) {
    regionSelect.querySelector(`option[value="${CSS.escape(rejected)}"]`)?.remove();
  }
  regionSelect.value = "";
  regionWarning.hidden = false;
  regionWarning.textContent = PLACE_UNAVAILABLE;
}

function finishWork(): void {
  if (!workWasRunning) {
    return;
  }
  workWasRunning = false;
  activeWork = "";
}

function presentActivity(next: Deployment): void {
  const status = next.status;
  if (next.running) {
    workWasRunning = true;
    const domainWork = activeWork === "domain" || (activeWork === "" && isDomainSecurityMessage(status));
    if (domainWork) {
      domainStatus.textContent = status;
      deployStatus.textContent = "";
    } else {
      deployStatus.textContent = status || WORKING;
      domainStatus.textContent = "";
    }
    shownStatus = status;
    return;
  }
  if (status && status !== shownStatus && status !== READY) {
    shownStatus = status;
    const plain = plainMessage(status);
    logError("setup", status);
    deployStatus.textContent = "";
    domainStatus.textContent = "";
    if (activeWork === "domain" || isDomainSecurityMessage(status)) {
      showDomainError(plain);
    } else {
      showDeployError(plain);
      if (plain === PLACE_UNAVAILABLE) {
        askForAnotherPlace(regionSelect.value || next.region);
      }
    }
    finishWork();
    return;
  }
  deployStatus.textContent = "";
  domainStatus.textContent = "";
  shownStatus = status;
  finishWork();
}

function renderDeployment(next: Deployment): void {
  deployment = next;
  const hasToken = next.hasDigitalOceanToken;
  const hasResend = next.hasResendApiKey;
  const sendingAddress = sendingAddressOf(next.fromEmail);
  const hasSendingAddress = sendingAddress.length > 0;
  const signInEmail = chosenEmail(next.adminEmail);
  const hasSignInEmail = signInEmail.length > 0;
  const ownServer = next.host === "server";
  const hasServerLogin = ownServer && next.publicIp.length > 0 && next.hasSshKey && next.sshUser.length > 0;
  const ready = next.installed && !next.running;
  if (!hasToken && !ownServer) {
    tokenEditorOpen = true;
  }
  if (ownServer && !hasServerLogin) {
    serverEditorOpen = true;
  }
  if (!hasResend) {
    resendEditorOpen = true;
  }
  if (!hasSendingAddress) {
    fromEmailEditorOpen = true;
  }
  if (!hasSignInEmail) {
    adminEmailEditorOpen = true;
  }
  tokenState.textContent = hasToken ? "Saved on this computer" : "Still needed";
  resendState.textContent = hasResend ? "Saved on this computer" : "Still needed";
  fromEmailState.textContent = hasSendingAddress ? sendingAddress : "Still needed";
  adminEmailState.textContent = hasSignInEmail ? signInEmail : "Still needed";
  fromEmailFollowup.textContent = next.installed
    ? "After you save a new address, choose Update my app so the running app sends from it."
    : "People will see this address on messages from the app.";
  adminEmailFollowup.textContent = next.installed
    ? "After you save a new address, choose Update my app so the running app uses it."
    : "You will sign in with this address and the password we show when the app is ready.";
  dropletState.textContent = next.installed
    ? "Ready"
    : ownServer
      ? hasServerLogin
        ? "Saved"
        : "Not added yet"
      : "Not created yet";
  hostState.textContent = ownServer ? "Your computer" : "DigitalOcean";
  hostDigitalOcean.checked = !ownServer;
  hostServer.checked = ownServer;
  const lockHost = next.running || next.installed || actionLocked;
  hostDigitalOcean.disabled = lockHost;
  hostServer.disabled = lockHost;
  serverState.textContent = hasServerLogin ? `${next.sshUser} at ${next.publicIp}` : "Still needed";
  editServer.hidden = !hasServerLogin;
  removeServer.hidden = !hasServerLogin || next.installed;
  serverEditor.hidden = !ownServer || !serverEditorOpen;
  if (document.activeElement !== serverIp && (serverIp.value === "" || serverIp.value === shownServerIp)) {
    serverIp.value = ownServer ? next.publicIp : serverIp.value;
    if (ownServer) {
      shownServerIp = next.publicIp;
    }
  }
  if (document.activeElement !== serverUser && (serverUser.value === "" || serverUser.value === shownServerUser)) {
    serverUser.value = ownServer ? next.sshUser : serverUser.value;
    if (ownServer) {
      shownServerUser = next.sshUser;
    }
  }
  serverKey.value = chosenKeyPath
    ? (chosenKeyPath.split(/[/\\]/).pop() ?? chosenKeyPath)
    : next.hasSshKey
      ? "Saved on this computer"
      : "";
  releaseNote.textContent = releaseText(next);
  editToken.textContent = hasToken ? "Change" : "Add";
  editResend.textContent = hasResend ? "Change" : "Add";
  editFromEmail.textContent = hasSendingAddress ? "Change" : "Add";
  editFromEmail.hidden = !hasSendingAddress;
  editAdminEmail.textContent = hasSignInEmail ? "Change" : "Add";
  editAdminEmail.hidden = !hasSignInEmail;
  removeToken.hidden = !hasToken;
  removeResend.hidden = !hasResend;
  tokenPanel.hidden = ownServer;
  serverPanel.hidden = !ownServer;
  tokenEditor.hidden = ownServer || !tokenEditorOpen;
  resendEditor.hidden = !resendEditorOpen;
  fromEmailEditor.hidden = !fromEmailEditorOpen;
  adminEmailEditor.hidden = !adminEmailEditorOpen;
  const readyToCreate =
    (ownServer ? hasServerLogin : hasToken) && hasResend && hasSendingAddress && hasSignInEmail;
  regionStep.hidden = next.installed || !readyToCreate;
  placeFields.hidden = ownServer;
  serverDeployNote.hidden = !ownServer;
  dropletActions.hidden = !next.installed;
  deleteButton.textContent = ownServer ? "Forget this computer" : "Remove this computer";
  finished.hidden = !ready;
  const domainBusy =
    next.running && (activeWork === "domain" || (activeWork === "" && isDomainSecurityMessage(next.status)));
  domainPanel.hidden = !next.installed || (next.running && !domainBusy);
  runningActions.hidden = !ready;
  if (
    next.appUrl &&
    document.activeElement !== pageUrl &&
    (pageUrl.value === "" || pageUrl.value === shownAppUrl)
  ) {
    pageUrl.value = next.appUrl;
    shownAppUrl = next.appUrl;
  }
  if (ready) {
    const password = next.adminPassword ? ` The password to sign in is ${next.adminPassword}.` : "";
    const address = signInEmail ? ` Sign in with ${signInEmail}.` : "";
    const browser = `People on your team can open the app in a browser at ${next.appUrl}.`;
    finishedDetail.textContent = `${browser}${address}${password} Keep the password private.`;
  }
  const busy = next.running;
  deployButton.disabled = busy;
  redeployButton.disabled = busy || actionLocked;
  deleteButton.disabled = busy || actionLocked;
  removeToken.disabled = busy || actionLocked;
  removeResend.disabled = busy || actionLocked;
  editFromEmail.disabled = busy || actionLocked;
  saveFromEmail.disabled = busy || actionLocked;
  editAdminEmail.disabled = busy || actionLocked;
  saveAdminEmail.disabled = busy || actionLocked;
  editServer.disabled = busy || actionLocked;
  removeServer.disabled = busy || actionLocked;
  saveServer.disabled = busy || actionLocked;
  importKey.disabled = busy || actionLocked;
  const domainName = next.domain ?? "";
  const httpsReady = next.httpsReady === true;
  if (document.activeElement !== domainInput && (domainInput.value === "" || domainInput.value === shownDomain)) {
    domainInput.value = domainName;
    shownDomain = domainName;
  }
  showDomainRecords(domainName, domainInput.value, next.publicIp);
  httpsState.textContent = httpsReady
    ? `People can open https://${next.appHost}. The app calls https://${next.apiHost}.`
    : "";
  enableHttpsButton.textContent = httpsReady ? "Secure the address again" : "The rows are in place";
  saveDomainButton.disabled = next.running || actionLocked;
  enableHttpsButton.disabled = next.running || actionLocked;
  downloadBackup.disabled = busy || actionLocked;
  downloadConfig.disabled = busy || actionLocked;
  presentActivity(next);
  appName.textContent = "Buho";
  summary.textContent =
    "This window puts your app on the internet. Each section explains what we need. What you save stays on this computer, and you can change or remove it later.";
}

function releaseText(next: Deployment): string {
  if (releaseError) {
    return releaseError;
  }
  if (!availableRelease) {
    return "";
  }
  const newest = `The newest service is ${availableRelease.apiVersion}. The newest website is ${availableRelease.uiVersion}.`;
  const runningKnown = next.installed && next.apiVersion.length > 0 && next.uiVersion.length > 0;
  if (!runningKnown) {
    return newest;
  }
  if (next.apiVersion === availableRelease.apiVersion && next.uiVersion === availableRelease.uiVersion) {
    return `This computer is running service ${next.apiVersion} and website ${next.uiVersion}.`;
  }
  return `This computer is running service ${next.apiVersion} and website ${next.uiVersion}. ${newest} Choose Update my app to get the newest.`;
}

function chosenEmail(value: string): string {
  const email = value.trim().toLowerCase();
  const parts = email.split("@");
  if (parts.length !== 2 || /\s/.test(email)) {
    return "";
  }
  const [name, domain] = parts;
  const dot = domain.lastIndexOf(".");
  const ending = domain.slice(dot + 1);
  if (!name || dot <= 0 || ending.length < 2) {
    return "";
  }
  return email;
}

function sendingAddressOf(value: string): string {
  const email = chosenEmail(value);
  if (!email || email === "onboarding@resend.dev") {
    return DEFAULT_FROM_EMAIL;
  }
  return email;
}

function credentialInput() {
  return {
    digitalOceanToken: doToken.value.trim(),
    resendApiKey: resendKey.value.trim(),
    fromEmail: "",
    adminEmail: "",
    region: regionSelect.value,
    size: DROPLET_SIZE,
  };
}

async function chooseHost(host: "digitalocean" | "server"): Promise<void> {
  if (!deployment || deployment.running || deployment.installed || actionLocked) {
    if (deployment) {
      renderDeployment(deployment);
    }
    return;
  }
  if ((deployment.host === "server" ? "server" : "digitalocean") === host) {
    return;
  }
  try {
    const next = await invoke<Deployment>("save_host", { host });
    renderDeployment(next);
    if (next.host !== "server" && next.hasDigitalOceanToken) {
      await loadRegions();
    }
  } catch (error) {
    logError("where the app runs", error);
    showDeployError(plainMessage(String(error)));
    if (deployment) {
      renderDeployment(deployment);
    }
  }
}

async function saveCredentials(patch: Partial<ReturnType<typeof credentialInput>>): Promise<void> {
  renderDeployment(
    await invoke<Deployment>("save_deployment", {
      input: { ...credentialInput(), ...patch },
    }),
  );
}

async function loadRegions(): Promise<void> {
  if (!deployment?.hasDigitalOceanToken) {
    return;
  }
  const regions = await invoke<RegionChoice[]>("droplet_regions");
  const previous = regionSelect.value;
  regionSelect.replaceChildren();
  const placeholder = document.createElement("option");
  placeholder.value = "";
  placeholder.textContent = "Choose a place";
  regionSelect.append(placeholder);
  for (const region of regions) {
    const option = document.createElement("option");
    option.value = region.slug;
    option.textContent = region.name;
    regionSelect.append(option);
  }
  const saved = deployment.region;
  const savedIsOffered = regions.some((region) => region.slug === saved);
  if (regions.length === 0) {
    regionSelect.value = "";
    regionWarning.hidden = false;
    regionWarning.textContent =
      "DigitalOcean did not offer any places we can use. Check that the key has Full Access, then try again.";
  } else if (previous && regions.some((region) => region.slug === previous)) {
    regionSelect.value = previous;
  } else if (!savedIsOffered && saved.length > 0 && (deployment.dropletId === 0 || deployment.publicIp.length === 0)) {
    regionSelect.value = "";
    regionWarning.hidden = false;
    regionWarning.textContent =
      "The place saved on this computer is not available anymore. Choose another place close to your team before you continue.";
  } else {
    regionSelect.value = "";
  }
  deployButton.disabled = deployment.running;
}

async function loadRelease(): Promise<void> {
  try {
    availableRelease = await invoke<ReleaseVersions>("available_versions");
    releaseError = "";
  } catch (error) {
    logError("newest versions", error);
    if (!availableRelease) {
      releaseError = plainMessage(String(error));
    }
  }
}

export function watchForNewerRelease(): void {
  window.setInterval(() => {
    if (deploymentIsRunning()) {
      return;
    }
    void loadRelease().then(() => {
      if (deployment) {
        renderDeployment(deployment);
      }
    });
  }, 15 * 60 * 1000);
}

function showAppUpdate(notice: AppUpdate): void {
  if (!notice.updateAvailable || notice.downloadUrl.length === 0) {
    buhoUpdate.hidden = true;
    appDownloadUrl = "";
    return;
  }
  appDownloadUrl = notice.downloadUrl;
  buhoUpdateText.textContent = `A newer Buho is ready. This copy is ${notice.currentVersion}. The newest is ${notice.latestVersion}. Download it, open the installer, and replace this copy.`;
  buhoUpdate.hidden = false;
}

async function loadAppUpdate(): Promise<void> {
  try {
    showAppUpdate(await invoke<AppUpdate>("app_update"));
  } catch (error) {
    logError("buho update", error);
  }
}

export function watchForAppUpdate(): void {
  void loadAppUpdate();
  window.setInterval(() => {
    void loadAppUpdate();
  }, 15 * 60 * 1000);
}

export async function refreshDeployment(): Promise<void> {
  const next = await invoke<Deployment>("deployment_state");
  if (sawRunning && !next.running) {
    releaseLoaded = false;
  }
  sawRunning = next.running;
  renderDeployment(next);
  if (!next.running && !releaseLoaded) {
    releaseLoaded = true;
    await loadRelease();
    if (deployment) {
      renderDeployment(deployment);
    }
  }
  if (next.host !== "server" && next.hasDigitalOceanToken && regionSelect.options.length === 0) {
    await loadRegions();
  }
}

async function forget(kind: string): Promise<void> {
  if (kind === "token") {
    tokenError.textContent = "";
  }
  if (kind === "resend") {
    resendError.textContent = "";
  }
  renderDeployment(await invoke<Deployment>("forget_resource", { kind }));
}

function setPageError(message: string): void {
  pageError.textContent = message;
  pageUrl.setAttribute("aria-invalid", message ? "true" : "false");
}

async function openPage(raw: string): Promise<void> {
  const address = raw.trim();
  if (!address) {
    setPageError("Type a full address, such as https://example.com.");
    return;
  }
  setPageError("");
  try {
    await invoke("open_in_app", { url: address });
    pageUrl.value = address;
  } catch (error) {
    logError("open page", error);
    setPageError(plainMessage(String(error)));
  }
}

export function mountDeploy(): void {
  pageForm.addEventListener("submit", (event) => {
    event.preventDefault();
    void openPage(pageUrl.value);
  });
  document.querySelector("#open-running-app")?.addEventListener("click", () => {
    void openPage(deployment?.appUrl ?? pageUrl.value);
  });
  document.querySelector("#open-running-app-finished")?.addEventListener("click", () => {
    void openPage(deployment?.appUrl ?? pageUrl.value);
  });
  document.querySelector("#create-token")?.addEventListener("click", () => {
    void invoke("open_external", { url: TOKEN_URL });
  });
  document.querySelector("#create-resend")?.addEventListener("click", () => {
    void invoke("open_external", { url: RESEND_URL });
  });
  buhoUpdateDownload.addEventListener("click", () => {
    if (!appDownloadUrl) {
      return;
    }
    void invoke("open_external", { url: appDownloadUrl }).catch((error: unknown) => {
      logError("buho update", error);
      buhoUpdateText.textContent =
        "We could not open the download page. Open buho.tech-fa.ca in a browser and download the newest Buho.";
    });
  });
  regionSelect.addEventListener("change", () => {
    if (regionSelect.value) {
      regionWarning.hidden = true;
    }
  });

  hostDigitalOcean.addEventListener("change", () => {
    if (hostDigitalOcean.checked) {
      void chooseHost("digitalocean");
    }
  });
  hostServer.addEventListener("change", () => {
    if (hostServer.checked) {
      void chooseHost("server");
    }
  });
  editServer.addEventListener("click", () => {
    serverEditorOpen = Boolean(serverEditor.hidden);
    if (serverEditorOpen && deployment) {
      serverIp.value = deployment.publicIp;
      serverUser.value = deployment.sshUser;
      shownServerIp = deployment.publicIp;
      shownServerUser = deployment.sshUser;
    }
    serverEditor.hidden = !serverEditorOpen;
  });
  importKey.addEventListener("click", async () => {
    serverError.textContent = "";
    const selected = await open({
      multiple: false,
      title: "Choose the private key",
    });
    if (typeof selected !== "string") {
      return;
    }
    chosenKeyPath = selected;
    serverKey.value = selected.split(/[/\\]/).pop() ?? selected;
  });
  saveServer.addEventListener("click", async () => {
    serverError.textContent = "";
    const ip = serverIp.value.trim();
    const user = serverUser.value.trim();
    if (!ip) {
      serverError.textContent = "Type the IP address of the computer.";
      return;
    }
    if (!chosenKeyPath && !deployment?.hasSshKey) {
      serverError.textContent = "Choose the private key file.";
      return;
    }
    try {
      serverEditorOpen = false;
      const next = await invoke<Deployment>("save_own_server", {
        ip,
        user,
        keyPath: chosenKeyPath,
      });
      chosenKeyPath = "";
      shownServerIp = next.publicIp;
      shownServerUser = next.sshUser;
      serverIp.value = next.publicIp;
      serverUser.value = next.sshUser;
      renderDeployment(next);
    } catch (error) {
      logError("your computer", error);
      serverEditorOpen = true;
      serverError.textContent = plainMessage(String(error));
      if (deployment) {
        renderDeployment(deployment);
      }
    }
  });
  removeServer.addEventListener("click", async () => {
    if (!deployment || actionLocked || deployment.installed) {
      return;
    }
    actionLocked = true;
    renderDeployment(deployment);
    try {
      const accepted = await confirm(
        "Remove the address and key saved for this computer? The computer itself stays as it is.",
        { title: "Remove this computer", kind: "warning", okLabel: "Remove", cancelLabel: "Keep it" },
      );
      if (!accepted) {
        return;
      }
      chosenKeyPath = "";
      shownServerIp = "";
      shownServerUser = "";
      serverIp.value = "";
      serverUser.value = "";
      renderDeployment(await invoke<Deployment>("clear_own_server"));
    } catch (error) {
      logError("your computer", error);
      serverError.textContent = plainMessage(String(error));
    } finally {
      actionLocked = false;
      if (deployment) {
        renderDeployment(deployment);
      }
    }
  });

  editToken.addEventListener("click", () => {
    tokenEditorOpen = Boolean(tokenEditor.hidden);
    tokenEditor.hidden = !tokenEditorOpen;
  });
  editResend.addEventListener("click", () => {
    resendEditorOpen = Boolean(resendEditor.hidden);
    resendEditor.hidden = !resendEditorOpen;
  });
  editFromEmail.addEventListener("click", () => {
    fromEmailEditorOpen = Boolean(fromEmailEditor.hidden);
    if (fromEmailEditorOpen && deployment) {
      fromEmail.value = sendingAddressOf(deployment.fromEmail);
    }
    fromEmailEditor.hidden = !fromEmailEditorOpen;
  });
  saveFromEmail.addEventListener("click", async () => {
    fromEmailError.textContent = "";
    const email = sendingAddressOf(fromEmail.value);
    if (!email) {
      fromEmailError.textContent = "Type the address people should see when the app emails them.";
      return;
    }
    try {
      fromEmailEditorOpen = false;
      await saveCredentials({ fromEmail: email });
      fromEmail.value = "";
      deployStatus.textContent = deployment?.publicIp
        ? "Saved. Choose Update my app so the running app sends from this address."
        : "Saved. We will use this address when we make your app.";
    } catch (error) {
      logError("sending address", error);
      fromEmailError.textContent = plainMessage(String(error));
    }
  });
  editAdminEmail.addEventListener("click", () => {
    adminEmailEditorOpen = Boolean(adminEmailEditor.hidden);
    if (adminEmailEditorOpen && deployment) {
      adminEmail.value = chosenEmail(deployment.adminEmail);
    }
    adminEmailEditor.hidden = !adminEmailEditorOpen;
  });
  saveAdminEmail.addEventListener("click", async () => {
    adminEmailError.textContent = "";
    const email = chosenEmail(adminEmail.value);
    if (!email) {
      adminEmailError.textContent = "Type an email address you can open, such as the one you use for work.";
      return;
    }
    try {
      adminEmailEditorOpen = false;
      await saveCredentials({ adminEmail: email });
      adminEmail.value = "";
      deployStatus.textContent = deployment?.publicIp
        ? "Saved. Choose Update my app so the running app uses this sign-in email."
        : "Saved. We will use this email when we make your app.";
    } catch (error) {
      logError("sign-in email", error);
      adminEmailError.textContent = plainMessage(String(error));
    }
  });

  removeToken.addEventListener("click", async () => {
    if (!deployment || actionLocked) {
      return;
    }
    actionLocked = true;
    renderDeployment(deployment);
    try {
      const accepted = await confirm(
        "Remove the DigitalOcean key from this computer? DigitalOcean will still accept that key until you delete it on their website.",
        { title: "Remove the DigitalOcean key", kind: "warning", okLabel: "Remove", cancelLabel: "Keep it" },
      );
      if (!accepted) {
        return;
      }
      await forget("token");
      doToken.value = "";
    } catch (error) {
      logError("remove the DigitalOcean key", error);
      tokenError.textContent = plainMessage(String(error));
    } finally {
      actionLocked = false;
      if (deployment) {
        renderDeployment(deployment);
      }
    }
  });

  removeResend.addEventListener("click", async () => {
    if (!deployment || actionLocked) {
      return;
    }
    actionLocked = true;
    renderDeployment(deployment);
    try {
      const accepted = await confirm("Remove the email key from this computer?", {
        title: "Remove the email key",
        kind: "warning",
        okLabel: "Remove",
        cancelLabel: "Keep it",
      });
      if (!accepted) {
        return;
      }
      await forget("resend");
      resendKey.value = "";
    } catch (error) {
      logError("remove the email key", error);
      resendError.textContent = plainMessage(String(error));
    } finally {
      actionLocked = false;
      if (deployment) {
        renderDeployment(deployment);
      }
    }
  });

  document.querySelector("#save-token")?.addEventListener("click", async () => {
    tokenError.textContent = "";
    const token = doToken.value.trim();
    try {
      await invoke("check_token", { token });
      tokenEditorOpen = false;
      await saveCredentials({ digitalOceanToken: token });
      doToken.value = "";
      await loadRegions();
    } catch (error) {
      logError("DigitalOcean key", error);
      tokenError.textContent = plainMessage(String(error));
    }
  });

  document.querySelector("#save-resend")?.addEventListener("click", async () => {
    resendError.textContent = "";
    const key = resendKey.value.trim();
    if (!key) {
      resendError.textContent = "Paste the email key from Resend.";
      return;
    }
    try {
      resendEditorOpen = false;
      await saveCredentials({ resendApiKey: key });
      resendKey.value = "";
      deployStatus.textContent = deployment?.publicIp
        ? "Saved. Choose Update my app so the running app uses the new email key."
        : "Saved. We will use this email key when we make your app.";
    } catch (error) {
      logError("email key", error);
      resendError.textContent = plainMessage(String(error));
    }
  });

  deployButton.addEventListener("click", async () => {
    const ownServer = deployment?.host === "server";
    if (!ownServer && !regionSelect.value) {
      regionWarning.hidden = false;
      regionWarning.textContent = "Choose a place close to your team, then continue.";
      return;
    }
    regionWarning.hidden = true;
    try {
      await saveCredentials(ownServer ? {} : { region: regionSelect.value });
      dismissedDeployError.value = "";
      activeWork = "deploy";
      renderDeployment(await invoke<Deployment>("start_deploy"));
    } catch (error) {
      logError("make the app", error);
      activeWork = "";
      const plain = plainMessage(String(error));
      showDeployError(plain);
      if (plain === PLACE_UNAVAILABLE) {
        askForAnotherPlace(regionSelect.value);
      }
    }
  });

  redeployButton.addEventListener("click", async () => {
    dismissedDeployError.value = "";
    activeWork = "deploy";
    try {
      renderDeployment(await invoke<Deployment>("start_deploy"));
    } catch (error) {
      logError("update the app", error);
      activeWork = "";
      showDeployError(plainMessage(String(error)));
    }
  });

  domainInput.addEventListener("input", () => {
    showDomainRecords(deployment?.domain ?? "", domainInput.value, deployment?.publicIp ?? "");
  });

  saveDomainButton.addEventListener("click", async () => {
    dismissedDomainError.value = "";
    const domain = domainInput.value.trim();
    if (!domain) {
      showDomainError("Type the domain you own, such as example.com.");
      return;
    }
    try {
      const next = await invoke<Deployment>("save_domain", { domain });
      shownDomain = next.domain;
      domainInput.value = next.domain;
      renderDeployment(next);
      domainStatus.textContent = "";
    } catch (error) {
      logError("domain", error);
      showDomainError(plainMessage(String(error)));
    }
  });

  enableHttpsButton.addEventListener("click", async () => {
    dismissedDomainError.value = "";
    activeWork = "domain";
    const typed = domainInput.value.trim().toLowerCase().replace(/\.+$/, "");
    if (typed !== deployment?.domain) {
      activeWork = "";
      showDomainError("Choose Add domain first. The rows have to match the domain you added.");
      return;
    }
    try {
      renderDeployment(await invoke<Deployment>("start_https"));
    } catch (error) {
      logError("secure the domain", error);
      activeWork = "";
      showDomainError(plainMessage(String(error)));
    }
  });

  deleteButton.addEventListener("click", async () => {
    if (!deployment || actionLocked) {
      return;
    }
    actionLocked = true;
    dismissedDeployError.value = "";
    renderDeployment(deployment);
    try {
      const ownServer = deployment.host === "server";
      const accepted = await confirm(
        ownServer
          ? "Forget this computer in Buho? The app keeps running there. We only remove the address, user name, and key saved on this computer."
          : "Remove the computer that runs your app? The app will stop working there. The keys saved on this computer stay, so you can create it again.",
        {
          title: ownServer ? "Forget this computer" : "Remove this computer",
          kind: "warning",
          okLabel: ownServer ? "Forget" : "Remove",
          cancelLabel: "Keep it",
        },
      );
      if (!accepted) {
        return;
      }
      chosenKeyPath = "";
      shownServerIp = "";
      shownServerUser = "";
      serverIp.value = "";
      serverUser.value = "";
      renderDeployment(await invoke<Deployment>("delete_droplet"));
      if (deployment?.host !== "server") {
        await loadRegions();
      }
    } catch (error) {
      logError("remove the computer", error);
      showDeployError(plainMessage(String(error)));
    } finally {
      actionLocked = false;
      if (deployment) {
        renderDeployment(deployment);
      }
    }
  });

  downloadConfig.addEventListener("click", async () => {
    downloadNote.textContent = "";
    downloadError.textContent = "";
    const destination = await save({
      title: "Save a copy of your setup",
      defaultPath: "my-app-setup.json",
      filters: [{ name: "Setup file", extensions: ["json"] }],
    });
    if (!destination) {
      return;
    }
    try {
      await invoke("export_configuration", { destination });
      downloadNote.textContent = "Saved. Keep that file somewhere private, and do not share it.";
    } catch (error) {
      logError("save a copy", error);
      downloadError.textContent = plainMessage(String(error));
    }
  });

  downloadBackup.addEventListener("click", async () => {
    backupNote.textContent = "";
    backupError.textContent = "";
    const destination = await save({
      title: "Download your database and files",
      defaultPath: "buho-backup.zip",
      filters: [{ name: "Zip file", extensions: ["zip"] }],
    });
    if (!destination || !deployment || actionLocked) {
      return;
    }
    actionLocked = true;
    renderDeployment(deployment);
    backupNote.textContent =
      "Downloading your database and files. This can take a while. Leave this window open.";
    let failure = "";
    try {
      await invoke("download_backup", { destination });
      backupNote.textContent = "Saved. Keep that file somewhere private, and do not share it.";
    } catch (error) {
      logError("download data", error);
      backupNote.textContent = "";
      failure = plainMessage(String(error));
    } finally {
      actionLocked = false;
      if (deployment) {
        renderDeployment(deployment);
      }
    }
    if (failure) {
      backupError.textContent = failure;
    }
  });

  deployErrorClose.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    closeErrorBanner(deployErrorBanner, deployErrorText, dismissedDeployError);
  });
  domainErrorClose.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    closeErrorBanner(domainErrorBanner, domainErrorText, dismissedDomainError);
  });
}

export function deploymentIsRunning(): boolean {
  return deployment?.running === true;
}
