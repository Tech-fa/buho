# Buho

A small desktop window that serves a built web app on this computer and writes the settings that app asks for.

The window is the setup screen. The web app is a folder of static files. That folder includes `shell.manifest.json`, which names the settings. Buho saves them on this computer and serves them at the path the app already fetches (`/config.json` for the dashboard). The same project runs on macOS and Windows. Build the installer on each system.

## Run

```bash
cd desktop-shell
npm install
npm run tauri dev
```

The first time the app opens, it asks you to choose a web app bundle: the folder that contains `index.html`. For the dashboard, build it, then choose `4es_dashboard/dist`:

```bash
cd 4es_dashboard
npm run build
```

You can also point at a folder without the menu:

```bash
npm run tauri -- dev -- --ui ../4es_dashboard/dist
```

Or set `APP_SHELL_UI` to that folder.

Enter the app's public URL and choose **Open**. The Shell menu has **Setup**, **Open app**, and **Agent CLI**. Agent CLI opens a terminal on this computer. That terminal has `APP_MCP_URL` and `APP_TOOLS_URL`. Your agent's CLI connects to `APP_MCP_URL`. The tools `list_pages`, `navigate`, `describe_page`, `set_fields`, and `submit_form` run in the open window. The same list is posted at `APP_TOOLS_URL` and, for the dashboard, at `/app-tools.json`. A local bundle still works from the section below. The local pages are only served on `127.0.0.1`.

## What the web app ships

Put `shell.manifest.json` next to `index.html` (in Vite, that is the `public/` folder):

```json
{
  "id": "my-app",
  "name": "My App",
  "configFile": "config.json",
  "spa": true,
  "fields": [
    {
      "key": "apiUrl",
      "label": "API URL",
      "type": "url",
      "required": true,
      "placeholder": "http://127.0.0.1:3002"
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `id` | Stable name for the saved settings. Same id shares settings across folders. |
| `name` | Title shown in the setup window. |
| `configFile` | File the running app fetches. Must stay inside the build folder. |
| `spa` | Unknown paths without a file extension serve `index.html`. |
| `fields` | Settings to ask for. `type` is `string`, `url`, `number`, `boolean`, or `text`. |

A folder with no manifest still runs. Settings are inferred from an existing `config.json`, and you can add keys by hand. Those settings are not declared by the app, so a manifest is the contract to ship.

Saved settings go in the OS app-data folder and override `config.json` inside the build. Replacing the build folder does not wipe them.
