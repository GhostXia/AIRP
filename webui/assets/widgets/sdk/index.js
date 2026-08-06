// C-P4-4（审计 #489 W3）：SDK 示例包的安装入口。
//
// engine 安装面要求包内必须含 index.js（entry point）：安装校验
// （engine/src/extensions/mod.rs validate_and_decode_files）拒绝无 index.js
// 的包，且第三方 esm 安装后 entry.source 一律被强制改写为
// /extensions/<digest>/index.js。因此示例包以 index.js 作为加载入口，
// re-export example-widget.js 的默认工厂与 manifest——按 example-manifest.json
// 的 files 清单打包即可走通安装校验，无需额外构建步骤。
export { default, manifest } from './example-widget.js';
