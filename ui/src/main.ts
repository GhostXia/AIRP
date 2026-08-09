import { createApp } from "vue";
import App from "./App.vue";
import { setDefaultEsmImporter } from "./registry";
import { initEngineGrants } from "./registry/consent";
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

// #474：先向 engine 获取权威 grant 快照，再挂载 UI。失败仍启动界面，但
// consent 模块保持 fail-closed；localStorage 不得在桌面/standalone 间漂移。
void initEngineGrants().then(() => {
  createApp(App).mount("#app");
});
