import { logError } from "./log";
import { bootHome, refreshCatalog } from "./apps/home";
import { bootChrome } from "./chrome/shell";
import {
  deploymentIsRunning,
  mountDeploy,
  refreshDeployment,
  watchAppHealth,
  watchForAppUpdate,
  watchForNewerRelease,
} from "./setup/deploy";
import { bootWorkspace } from "./workspace/workspace";
import "./styles.css";

const summary = document.querySelector("#summary") as HTMLParagraphElement;

bootChrome();
bootHome();
bootWorkspace();
mountDeploy();

async function boot(): Promise<void> {
  let last = "This window is still opening.";
  for (let attempt = 0; attempt < 20; attempt += 1) {
    try {
      await refreshDeployment();
      await refreshCatalog();
      window.setInterval(() => {
        if (deploymentIsRunning()) {
          void refreshDeployment().catch((error: unknown) => {
            logError("setup", error);
          });
          void refreshCatalog();
        }
      }, 2000);
      watchForNewerRelease();
      watchForAppUpdate();
      watchAppHealth();
      return;
    } catch (error) {
      logError("open the window", error);
      last = String(error);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }
  if (summary) {
    summary.textContent = last;
  }
}

void boot();
