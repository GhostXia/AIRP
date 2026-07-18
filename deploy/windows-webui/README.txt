AIRP Windows WebUI preview
==========================

1. Double-click Start-AIRP.cmd.
2. Your default browser opens http://127.0.0.1:8765.
3. Complete onboarding and enter your own provider endpoint, API key, and model.
4. Keep the launcher window open while using AIRP. Closing it stops AIRP.

No Rust, Node.js, Docker, WSL, or Tauri installation is required.
All mutable AIRP files stay inside this extracted folder: user content is in
data\ and process configuration is in config.json. Back up data\ before an
upgrade, and copy the existing data\ into the new AIRP folder instead of
deleting or overwriting it. Protect this folder and your provider credentials.
Provider API keys intentionally remain process-only and must be entered again
after AIRP is restarted; the package does not write them to config or data.

Security boundary: this preview binds only to 127.0.0.1 and is for one user on
one Windows machine. Do not expose or proxy port 8765 to a LAN or the Internet.
