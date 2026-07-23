AIRP Linux WebUI preview
========================

1. Run ./start-airp.sh in a terminal. It does not need root, install anything,
   or modify your system.
2. Open http://127.0.0.1:8765 in your browser. The launcher does not auto-open
   a browser on Linux; the engine prints the URL on startup.
3. Complete onboarding and enter your own provider endpoint, API key, and model.
4. Keep the terminal open while using AIRP. Closing it or pressing Ctrl+C stops
   AIRP.

No Rust, Node.js, Docker, or Tauri installation is required. The airp-core
binary is statically linked against musl, so it runs on any x86_64 Linux
distribution without additional runtime libraries.

All mutable AIRP files stay inside this extracted folder: user content is in
data/ and process configuration is in config.json. Back up data/ before an
upgrade, and copy the existing data/ into the new AIRP folder instead of
deleting or overwriting it. Protect this folder and your provider credentials.
Provider API keys are stored in data/secrets.json and are intentionally not
returned by the API or shown again in the UI. This file is plaintext, matching
the transparent local-user tradeoff common to single-user local-first apps.
Anyone who can read this file can use the key, so do not share the AIRP folder,
publish it, or include secrets.json in support bundles.

Security boundary: this preview binds only to 127.0.0.1 and is for one user on
one machine. Do not expose or proxy port 8765 to a LAN or the Internet.
