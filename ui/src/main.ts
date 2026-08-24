import { createApp } from "vue";
import App from "./App.vue";
import { initEngineGrants } from "./registry/consent";
import { initEngineCatalog } from "./registry/engine-catalog";
import { consumeDesktopTokenFragment } from "./protocol/desktop-session";
import "./styles/desktop.css";

// The Tauri shell delivers the short-lived bearer in a fragment. Consume and
// erase it before any Engine-authoritative bootstrap request is made.
consumeDesktopTokenFragment();

// #474：先向 engine 获取权威 grant 快照（5s deadline），再挂载 UI。超时/失败
// 仍启动宿主界面，但 consent 模块保持 fail-closed；localStorage 不得在桌面/
// standalone 间漂移，第三方 widget 继续 gated。
void Promise.all([initEngineGrants(), initEngineCatalog()]).then(() => {
  createApp(App).mount("#app");
});
