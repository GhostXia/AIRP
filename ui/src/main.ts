import { createApp } from "vue";
import App from "./App.vue";
import { setDefaultEsmImporter } from "./registry";
import { initGrants } from "./registry/consent";
import statusPill from "./widgets/status.module";

// Map the demo's local esm source specifiers to in-repo modules so the third-
// party widget loads with no network/CDN. A real host would leave the default
// importer (dynamic import of the manifest `source`) untouched.
const LOCAL_ESM_SOURCES: Record<string, () => Promise<unknown>> = {
  "demo:acme/status-pill": async () => ({ default: statusPill }),
};
setDefaultEsmImporter((source) => {
  const loader = LOCAL_ESM_SOURCES[source];
  return loader ? loader() : import(/* @vite-ignore */ source);
});

// Restore previously saved widget consent grants from localStorage so the user
// does not have to re-approve every reload. Subsequent grant/revoke/clear calls
// auto-persist. No-op if storage is unavailable.
initGrants();

createApp(App).mount("#app");
