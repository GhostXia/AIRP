AIRP Windows WebUI preview (`v0.0.5-rc.2`, `main@affa315`)
============================================================

Candidate evidence (2026-08-09): GitHub Actions run 31309894372 completed
the Windows package, browser and desktop smoke jobs successfully and uploaded
the prerelease assets. This is a prerelease candidate, not formal v0.0.5;
real-provider/browser/Compose acceptance (#130) remains open.

1. Double-click Start-AIRP.cmd. It runs directly and does not use PowerShell,
   request administrator access, or install anything.
2. Your default browser opens http://127.0.0.1:8765.
3. Complete onboarding and enter your own provider endpoint, API key, and model.
4. Keep the launcher window open while using AIRP. Closing it stops AIRP.

Desktop UI (v0.0.4+): double-click airp-ui.exe instead of Start-AIRP.cmd to run
AIRP in a desktop window. Both entry points share this same folder: the same
airp-core.exe engine, the same webui\ assets, and the same data\ folder, so your
characters and sessions are identical either way. Do not run both at the same
time (each starts its own engine instance).

No Rust, Node.js, Docker, WSL, or Tauri installation is required.
All mutable AIRP files stay inside this extracted folder: user content is in
data\ and process configuration is in config.json. Back up data\ before an
upgrade, and copy the existing data\ into the new AIRP folder instead of
deleting or overwriting it. Protect this folder and your provider credentials.
Provider API keys are stored in data\secrets.json and are intentionally not
returned by the API or shown again in the UI. This file is plaintext, matching
the transparent local-user tradeoff used by projects such as SillyTavern.
Anyone who can read this file can use the key, so do not share the AIRP folder,
publish it, or include secrets.json in support bundles.

Security boundary: this preview binds only to 127.0.0.1 and is for one user on
one Windows machine. Do not expose or proxy port 8765 to a LAN or the Internet.
