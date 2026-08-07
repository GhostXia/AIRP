//! Trusted Plugin 基础设施（#498 §6）：manifest 解析、校验与加载。
//!
//! Trusted plugin 是用户显式安装的长跑子进程（与 widget 的 digest-pinned
//! 静态包不同：无 digest 锁、无 capability grant——显式信任模型，见
//! docs/TRUSTED-PLUGINS.md）。engine 只负责：扫描 manifest、spawn 子进程、
//! 反代 HTTP、退出时终止。安全边界 = 目录限定（command 必须在
//! `data/plugins/<id>/` 下）+ loopback 拓扑。

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub mod proxy;
pub mod spawn;

/// 单个 trusted plugin 的声明（`data/plugins/manifests/<id>.json`）。
///
/// 字段语义见 docs/TRUSTED-PLUGINS.md §2。`host_api` 与 widget 一致：
/// 纯 semver 字符串，major 钉死（`crate::extensions::HOST_API_MAJOR`）。
#[derive(Debug, Clone, Deserialize)]
pub struct TrustedPluginManifest {
    /// `ns.name` 形态唯一 id（同时是 `data/plugins/<id>/` 目录名）。
    pub id: String,
    /// 插件自身版本（仅展示用，engine 不做语义判断）。
    pub version: String,
    /// 相对 `data/plugins/<id>/` 的可执行文件路径。
    pub command: String,
    /// 启动参数；`${AIRP_PLUGIN_PORT}` 占位符替换为 manifest.port。
    #[serde(default)]
    pub args: Vec<String>,
    /// 插件自己监听的 loopback 端口（engine 不分配、不探活）。
    pub port: u16,
    /// 所需宿主合同 major（纯 semver，校验同 widget）。
    pub host_api: String,
}

/// 校验 manifest 的必填字段与宿主合同 major（fail-closed：坏字段拒绝加载，
/// 不让声明不完整的插件进入 spawn 面）。
pub fn validate_manifest(m: &TrustedPluginManifest) -> Result<(), String> {
    // id 是文件系统目录名与 URL 段：必须无路径分隔符，防 `../` 越界。
    if m.id.is_empty()
        || m.id.len() > 128
        || m.id.starts_with('.')
        || m.id.ends_with('.')
        || m.id.contains('/')
        || m.id.contains('\\')
    {
        return Err(format!(
            "plugin id must be 'ns.name' without path separators: {:?}",
            m.id
        ));
    }
    if m.version.is_empty() || m.version.len() > 64 {
        return Err(format!("plugin version must be 1..=64 chars: {:?}", m.version));
    }
    if m.command.is_empty() {
        return Err(format!("plugin {} command must not be empty", m.id));
    }
    if m.port == 0 {
        return Err(format!("plugin {} port must be 1..=65535", m.id));
    }
    // host_api major 钉死（前向兼容铁律，与 widget 安装面同一校验）。
    let declared = crate::extensions::parse_host_api_major(Some(&m.host_api))
        .map_err(|e| format!("plugin {} host_api invalid: {e}", m.id))?;
    if declared != crate::extensions::HOST_API_MAJOR {
        return Err(format!(
            "plugin {} requires host_api major {} but engine supports {}",
            m.id,
            declared,
            crate::extensions::HOST_API_MAJOR
        ));
    }
    Ok(())
}

/// 解析 command 为绝对可执行路径：canonical 限定在 `data/plugins/<id>/`
/// 目录内且是文件（复用 plugin_tool 的越界检查模式）。
pub fn resolve_command(data_root: &Path, m: &TrustedPluginManifest) -> Result<PathBuf, String> {
    let plugin_dir = data_root.join("plugins").join(&m.id);
    let canonical_dir = plugin_dir.canonicalize().map_err(|e| {
        format!(
            "plugin {} directory {} not found: {e}",
            m.id,
            plugin_dir.display()
        )
    })?;
    let candidate = canonical_dir.join(&m.command);
    let canonical = candidate.canonicalize().map_err(|e| {
        format!(
            "plugin {} command {} not found: {e}",
            m.id,
            candidate.display()
        )
    })?;
    if !canonical.starts_with(&canonical_dir) {
        return Err(format!(
            "plugin {} command {} escapes plugins/<id>/ directory",
            m.id,
            canonical.display()
        ));
    }
    if !canonical.is_file() {
        return Err(format!(
            "plugin {} command {} is not a file",
            m.id,
            canonical.display()
        ));
    }
    Ok(canonical)
}

/// 替换 args 中的 `${AIRP_PLUGIN_PORT}` 占位符为 manifest 声明的端口。
/// 未识别的 `${...}` 保持原样（由插件自己决定是否拒绝）。
pub fn resolve_args(m: &TrustedPluginManifest) -> Vec<String> {
    let port = m.port.to_string();
    m.args
        .iter()
        .map(|a| a.replace("${AIRP_PLUGIN_PORT}", &port))
        .collect()
}

/// 扫描 `data/plugins/manifests/*.json` 加载全部合法 manifest。
///
/// 单个文件坏 JSON / 校验失败 / id 重复 → warn 并跳过，不阻塞其余插件
/// （与 §6.5「端口冲突在 spawn 时暴露」同哲学：一个坏声明不拖垮整层）。
pub fn load_manifests(data_root: &Path) -> Vec<TrustedPluginManifest> {
    let manifests_dir = data_root.join("plugins").join("manifests");
    let entries = match std::fs::read_dir(&manifests_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(), // 目录不存在 = 未安装任何 trusted plugin
    };
    let mut out: Vec<TrustedPluginManifest> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(path = %path.display(), %e, "trusted plugin manifest unreadable, skipped");
                continue;
            }
        };
        let manifest: TrustedPluginManifest = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(path = %path.display(), %e, "trusted plugin manifest invalid JSON, skipped");
                continue;
            }
        };
        if let Err(e) = validate_manifest(&manifest) {
            tracing::warn!(path = %path.display(), %e, "trusted plugin manifest rejected");
            continue;
        }
        if out.iter().any(|m| m.id == manifest.id) {
            tracing::warn!(
                id = %manifest.id,
                "duplicate trusted plugin id, keeping first manifest"
            );
            continue;
        }
        out.push(manifest);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_manifest(data_root: &Path, id: &str, json: &str) {
        let dir = data_root.join("plugins").join("manifests");
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join(format!("{id}.json"))).unwrap();
        f.write_all(json.as_bytes()).unwrap();
    }

    fn sample(id: &str) -> String {
        format!(
            r#"{{"id": "{id}", "version": "1.0.0", "command": "./server", "args": ["--port", "${{AIRP_PLUGIN_PORT}}"], "port": 8765, "host_api": "1"}}"#
        )
    }

    #[test]
    fn parse_manifest_ok() {
        let m: TrustedPluginManifest =
            serde_json::from_str(&sample("com.example.tts")).unwrap();
        assert_eq!(m.id, "com.example.tts");
        assert_eq!(m.port, 8765);
        assert_eq!(m.args, vec!["--port", "${AIRP_PLUGIN_PORT}"]);
    }

    #[test]
    fn parse_manifest_args_optional() {
        let m: TrustedPluginManifest = serde_json::from_str(
            r#"{"id": "a.b", "version": "1", "command": "./x", "port": 1, "host_api": "1"}"#,
        )
        .unwrap();
        assert!(m.args.is_empty());
    }

    #[test]
    fn validate_rejects_bad_fields() {
        // 缺 version
        assert!(serde_json::from_str::<TrustedPluginManifest>(
            r#"{"id": "a.b", "command": "./x", "port": 1, "host_api": "1"}"#
        )
        .is_err());
        // port 0
        let m: TrustedPluginManifest = serde_json::from_str(
            r#"{"id": "a.b", "version": "1", "command": "./x", "port": 0, "host_api": "1"}"#,
        )
        .unwrap();
        assert!(validate_manifest(&m).is_err());
        // id 越界（路径分隔符）
        let m: TrustedPluginManifest = serde_json::from_str(
            r#"{"id": "../evil", "version": "1", "command": "./x", "port": 1, "host_api": "1"}"#,
        )
        .unwrap();
        assert!(validate_manifest(&m).is_err());
        // host_api 跨 major
        let m: TrustedPluginManifest = serde_json::from_str(
            r#"{"id": "a.b", "version": "1", "command": "./x", "port": 1, "host_api": "2.0"}"#,
        )
        .unwrap();
        assert!(validate_manifest(&m).is_err());
    }

    #[test]
    fn resolve_command_requires_plugin_dir_and_rejects_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let data_root = tmp.path();
        let plugin_dir = data_root.join("plugins").join("com.example.tts");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let m: TrustedPluginManifest =
            serde_json::from_str(&sample("com.example.tts")).unwrap();

        // 未建目录 → 拒绝
        let missing = TrustedPluginManifest {
            command: "./nope".into(),
            ..m.clone()
        };
        assert!(resolve_command(data_root, &missing).is_err());

        // 目录内文件 → 通过（canonicalize 后比较，Windows 下有 \\?\ 前缀）
        let exe = plugin_dir.join("server");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        assert_eq!(resolve_command(data_root, &m).unwrap(), exe.canonicalize().unwrap());

        // 绝对路径 escape → 拒绝
        let abs = TrustedPluginManifest {
            command: plugin_dir.join("..").join("..").join("evil.sh").to_string_lossy().into_owned(),
            ..m.clone()
        };
        assert!(resolve_command(data_root, &abs).is_err());
    }

    #[test]
    fn resolve_args_replaces_port_placeholder() {
        let m: TrustedPluginManifest =
            serde_json::from_str(&sample("com.example.tts")).unwrap();
        assert_eq!(
            resolve_args(&m),
            vec!["--port".to_string(), "8765".to_string()]
        );
        // 无占位符保持原样
        let plain = TrustedPluginManifest {
            args: vec!["--flag".into(), "x".into()],
            ..m
        };
        assert_eq!(resolve_args(&plain), vec!["--flag", "x"]);
    }

    #[test]
    fn load_manifests_skips_bad_and_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let data_root = tmp.path();
        // 目录不存在 → 空
        assert!(load_manifests(data_root).is_empty());

        write_manifest(data_root, "good", &sample("com.example.good"));
        write_manifest(data_root, "bad", "{ not json");
        write_manifest(data_root, "dup1", &sample("com.example.good"));
        write_manifest(data_root, "cross-major", r#"{"id": "com.example.x", "version": "1", "command": "./x", "port": 1, "host_api": "9"}"#);

        let loaded = load_manifests(data_root);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "com.example.good");
    }
}
