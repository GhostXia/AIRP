# AIRP Risk Register

## RR-001 · `card_path` local arbitrary file read

- **Status**: Accepted for Tauri desktop local sidecar only.
- **Surface**: `/v1/characters/import` with `card_path`.
- **Risk**: If exposed to untrusted callers, `card_path` lets the engine process read arbitrary absolute paths visible to that process.
- **Current control**: Tauri UI obtains paths through the official file dialog, sends only the selected path, and the engine then validates the file as PNG/JSON before import.
- **Rule**: `card_path` is allowed only for trusted local UI. Future web clients, third-party widgets, or remote engines must use multipart/streaming upload, with base64 only as a last fallback. Do not expose server-side arbitrary path read to untrusted callers.
- **Future hardening**: Add engine-side caller capability checks before any multi-client/web exposure.
