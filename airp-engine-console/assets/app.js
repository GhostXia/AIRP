/* ============================================================
 * AIRP 控制台 · 样板导航贴片（app.js）
 * 在每个屏页面右下角注入一个返回导航首页的浮动贴片。
 * 该贴片是【样板脚手架】，不属于设计稿内容（带 data-sample-chrome），
 * 派生实现（WebUI/桌面端）不应携带。
 * ============================================================ */

(function () {
  const here = location.pathname.replace(/\\/g, "/");
  const cur = (typeof AIRP_SCREENS !== "undefined")
    ? AIRP_SCREENS.find(s => here.endsWith("/" + s.file) || here.endsWith("/" + s.file.split("/").pop()))
    : null;

  const chip = document.createElement("a");
  chip.href = "../index.html";
  chip.setAttribute("data-sample-chrome", "");
  chip.title = "返回样板导航首页（此贴片不属于设计稿）";
  chip.textContent = cur ? ("⌂ " + cur.id + " " + cur.title) : "⌂ 样板导航";
  Object.assign(chip.style, {
    position: "fixed",
    right: "16px",
    bottom: "16px",
    zIndex: 9999,
    padding: "7px 12px",
    fontSize: "11px",
    fontWeight: "600",
    fontFamily: "var(--font-body)",
    color: "#fff",
    background: "rgba(42,41,39,0.82)",
    borderRadius: "9999px",
    textDecoration: "none",
    boxShadow: "0 4px 12px rgba(0,0,0,0.18)",
    letterSpacing: "0.3px",
  });
  chip.onmouseenter = () => (chip.style.background = "rgba(42,41,39,0.95)");
  chip.onmouseleave = () => (chip.style.background = "rgba(42,41,39,0.82)");
  document.addEventListener("DOMContentLoaded", () => document.body.appendChild(chip));
})();
