# Desktop shell walkthrough

This is the native window that deploys the documents app and then shows it. The page you see is TypeScript. The work that touches disk, SSH, and DigitalOcean is Rust, because a web page cannot do those things.

## What happens when you open the app

1. `src-tauri/src/main.rs` starts Tauri, which calls `run()` in `src-tauri/src/lib.rs`.
2. `lib.rs` creates the app data folder and an empty `deployment.json` if this is the first launch.
3. It opens two local servers, both on `127.0.0.1` only:
   - the bundle server (`server.rs`), which can serve a local web build
   - the agent server (`control.rs`), which lets the agent CLI drive the window
4. The window loads `index.html`. `src/main.ts` starts the two screens and asks Rust for the current state.

## The window

`index.html` is the setup page. Styling is `src/styles.css`, using the same light page, white cards, and blue buttons as the dashboard.

`src/main.ts` does not contain the screens. It calls:

- `src/setup/bundle.ts` for the local bundle, its settings form, and the app URL
- `src/setup/deploy.ts` for the DigitalOcean steps
- `src/setup/types.ts` for the shapes those screens share

`deploy.ts` talks to Rust with `invoke`. The names match the functions marked `#[tauri::command]` in `lib.rs`:

| Button | Command | Rust |
|---|---|---|
| Continue after the token | `check_token`, then `save_deployment` | `deploy.rs` checks the token with DigitalOcean, then writes it |
| Continue after Resend | `save_deployment` | stores the Resend key |
| Region list | `droplet_regions` | `GET /v2/regions` |
| Deploy | `start_deploy` | runs `provision` on a background thread |
| Open the app | `open_public_app` | navigates the window to `http://<droplet ip>` |

The page never receives the DigitalOcean token, the Resend key, the SSH private key, or the database passwords again. Rust answers with `hasDigitalOceanToken` and `hasResendApiKey`. The admin password is the exception: it is shown after deploy so you can log in.

Until a droplet IP exists, the other panels stay hidden. The first screen is only the deploy steps.

## deployment.json

`deploy.rs` owns this file. It lives in the Tauri app data directory, not in the git repo. On macOS that is under `~/Library/Application Support/`.

The file is created empty. As you go through setup it gains:

- the DigitalOcean token and Resend key you pasted
- the region you picked
- a generated SSH keypair
- the droplet id, name, and public IP
- generated database, encryption, and admin passwords

The file mode is `0600`, so only your user can read it. An empty paste does not erase a token that is already saved. The droplet size is always `s-2vcpu-4gb`. The region has to look like a DigitalOcean slug (`nyc3`, `sfo3`).

## What Deploy does

`provision` in `deploy.rs` runs only after the token, Resend key, and region are present.

1. Generate an ed25519 key with `ssh-keygen` if one is not saved yet.
2. Register the public key with `POST /v2/account/keys`.
3. Create the droplet with `POST /v2/droplets` if one is not saved yet. The image is Ubuntu 24.04.
4. Poll until DigitalOcean returns a public IP, and reject anything that is not an IP address.
5. Wait until port 22 accepts a connection.
6. Pack `documents_backend`, `documents_dashboard`, and `desktop-shell/deploy` with `tar`. `node_modules`, `.git`, and `.env` are left out.
7. Upload that archive over SSH to `/opt/fourss`.
8. Upload a root-only `.env` for Compose, then `chmod 600` it on the droplet.
9. Run `desktop-shell/deploy/deploy.sh`.

`deploy.sh` installs Docker if the machine does not have it, writes the dashboard `config.json` so the UI calls `http://<ip>:3012`, and starts Compose.

`desktop-shell/deploy/docker-compose.yml` starts MySQL, Redis, RabbitMQ, the API, and the dashboard. The API image is `api.Dockerfile`. The dashboard image is `ui.Dockerfile`, served by nginx.

A second click on Deploy uses the saved droplet and key. It does not create another machine.

## The other Rust files

`settings.rs` is a different config from `deployment.json`. It is the settings of whatever web bundle you point the shell at (`shell.manifest.json` and that app's `config.json`). `server.rs` serves those files to the window and refuses paths that escape the bundle folder.

`terminal.rs` opens a local terminal for the agent. It receives the app URL and the agent-server URL. It does not receive the DigitalOcean token.

`control.rs` is that agent server. A page load of the opened app injects `bridge.js`, which can list pages, fill fields, and submit forms. The server listens on localhost and rejects a request that does not carry the random `token` query parameter. That token is given only to the agent terminal, as `APP_MCP_URL` and `APP_TOOLS_URL`. The setup page shows the server address without the token.

`capabilities/default.json` grants Tauri commands to the local shell windows. It does not grant them to the droplet's website. Navigating the window to the deployed app does not hand that site the shell's commands.

## What is still true after the review

- `deployment.json` is plaintext on disk. The protection is your user account and the `0600` mode, not encryption.
- The first SSH connection trusts the host key it sees (`accept-new`), then pins it in the shell's own `known_hosts` file.
- The agent server can click through the app you have open. Anything that learns the token from the agent terminal can do that too. A random website on the internet cannot, because it does not have the token and the port is not public.
- The shell window has no content-security policy, because the same window later loads the deployed app.
