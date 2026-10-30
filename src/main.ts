import { logError } from "./log";
import { deploymentIsRunning, mountDeploy, refreshDeployment, watchForNewerRelease } from "./setup/deploy";
import "./styles.css";

const summary = document.querySelector("#summary") as HTMLParagraphElement;

mountDeploy();

async function boot(): Promise<void> {
  let last = "This window is still opening.";
  for (let attempt = 0; attempt < 20; attempt += 1) {
    try {
      await refreshDeployment();
      window.setInterval(() => {
        if (deploymentIsRunning()) {
          void refreshDeployment().catch((error: unknown) => {
            logError("setup", error);
          });
        }
      }, 2000);
      watchForNewerRelease();
      return;
    } catch (error) {
      logError("open the window", error);
      last = String(error);
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }
  summary.textContent = last;
}

void boot();
