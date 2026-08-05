//! C-P2：engine 扩展注册面（extension registry / catalog）。
//!
//! 职责（获批计划 C-P2 章节 + Task #8 范围要点）：
//! 1. **manifest 校验**：第三方 widget 包的 WidgetDef 结构校验；第三方 esm
//!    强制 `entry.sandbox == true`（BUG-6 门禁第三道：render 层 → 注册面 →
//!    安装面，fail-closed）。
//! 2. **digest-pinned 静态包**：包清单含每个文件的内容摘要（SHA-256），安装时
//!    逐文件校验并落盘到 `data_root/extensions/<package_digest>/`；静态服务
//!    （`daemon::api` 的 `/extensions/{digest}/*`）按内容寻址只读投放，服务时
//!    再校验摘要防篡改。第三方包仅支持同源静态目录投放——安装时
//!    `entry.source` 一律强制改写为 `/extensions/<digest>/index.js`，跨源
//!    esm 无加载路径（叠加 frame CSP `script-src 'self'` 的天然阻断 = R0 硬门禁）。
//! 3. **catalog**：`GET /v1/extensions/catalog` 机器可读下发（manifests + slot
//!    计划），webui 从静态 slots.json 切换到 engine 权威下发（engine 无安装
//!    扩展时返回内置默认计划，webui 侧再降级为本地 slots.json，双保险不硬失败）。
//!
//! C-P3 预留：`WidgetManifest::capabilities` 与 intent 合同的 capability 字段
//! 在本阶段即是一等字段——C-P3 的权威授权（manifest ∩ 用户同意 ∩ engine
//! policy）与逐调用强制直接消费这些字段，无需重构数据结构。
//!
//! 持久化：`data_root/extensions.json`（记录清单）+ `data_root/extensions/`
//! （内容寻址包目录）。原子写：tmp + rename。
//!
//! LOCK-ORDER：`ExtensionStore::records` 是 daemon 级资源锁（§1.5），
//! 持锁期间只做内存操作；磁盘 I/O 在锁外完成后再短锁写回。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod api;

/// 单个包文件数上限（防 zip-bomb 式清单膨胀；widget 包本就只有几个文件）。
pub const MAX_PACKAGE_FILES: usize = 32;
/// 单个文件大小上限（1MB；widget 是文本资产，图片另走 CDN 不属本阶段）。
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
/// 包总大小上限（4MB）。
pub const MAX_PACKAGE_BYTES: usize = 4 * 1024 * 1024;

/// 校验错误：端点映射为 400 + `{ error: { code, message } }`。
#[derive(Debug)]
pub struct ValidationError {
    pub code: &'static str,
    pub message: String,
}

impl ValidationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// widget 加载入口（对译 webui/assets/widgets/manifests.js 的 entry 形状）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WidgetEntry {
    /// `esm`（第三方安装包唯一允许的形态；`builtin` 仅出现在内置默认 catalog）。
    pub kind: String,
    /// esm 加载源。安装时一律被强制改写为 `/extensions/<digest>/index.js`
    /// （R0 硬门禁：跨源加载路径在安装面即被消灭）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// 第三方 esm 必须为 true（BUG-6 反转默认在安装面再次强制）。
    #[serde(default)]
    pub sandbox: bool,
}

/// WidgetDef manifest（对译 webui manifests.js 消费的机器可读形状）。
///
/// C-P3 预留：`capabilities` 是权威授权的输入之一（manifest ∩ 用户同意 ∩
/// engine policy），本阶段即原样持久化与下发。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WidgetManifest {
    /// 全局唯一 widget 类型（如 `acme.clock`）。
    #[serde(rename = "type")]
    pub widget_type: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// 申请的 capabilities（C-P3 授权面输入；本阶段仅声明与透传）。
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub entry: WidgetEntry,
}

/// 包内单个文件的摘要锚（digest-pinned）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageFileMeta {
    pub path: String,
    /// 文件内容的 SHA-256 hex（小写）。安装时校验、服务时复检。
    pub sha256: String,
}

/// 已安装扩展记录（extensions.json 的一条）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRecord {
    pub id: String,
    /// widget 类型（同一 type 至多一条记录：重装即替换）。
    #[serde(rename = "type")]
    pub widget_type: String,
    /// 包级摘要：全部文件 `path:sha256` 按 path 排序拼接后的 SHA-256 hex。
    /// 同时是内容寻址目录名（`data_root/extensions/<digest>/`）。
    pub digest: String,
    pub installed_at: u64,
    pub enabled: bool,
    /// 安装时指定的挂载 slot（catalog 把它编入该 slot；须为内置计划已知 slot）。
    pub slot: String,
    pub manifest: WidgetManifest,
    pub files: Vec<PackageFileMeta>,
}

/// extensions.json 的落盘形状。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PersistedState {
    #[serde(default)]
    extensions: Vec<ExtensionRecord>,
}

/// 安装请求中的单个文件（内容 base64 + 声明摘要）。
#[derive(Debug, Deserialize)]
pub struct InstallFilePayload {
    pub path: String,
    pub content_base64: String,
    /// 声明者承诺的内容摘要；与实测不符即整包拒绝（digest-pinned）。
    pub sha256: String,
}

/// 安装请求体。
#[derive(Debug, Deserialize)]
pub struct InstallRequest {
    pub manifest: WidgetManifest,
    pub files: Vec<InstallFilePayload>,
    /// 挂载 slot（缺省 workbench.grid）；须为内置 slot 计划的已知 id。
    #[serde(default)]
    pub slot: Option<String>,
}

/// 扩展存储（daemon 单例，挂 `Arc<ExtensionStore>` 于 DaemonState）。
pub struct ExtensionStore {
    data_root: PathBuf,
    records: Mutex<Vec<ExtensionRecord>>,
}

impl ExtensionStore {
    /// 从 `data_root/extensions.json` 载入既有记录（缺失/损坏时从空起步，
    /// 损坏记 warn——扩展面损坏不得拖垮 daemon 启动）。
    pub fn load(data_root: PathBuf) -> Arc<Self> {
        let path = data_root.join("extensions.json");
        let extensions = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<PersistedState>(&text) {
                Ok(state) => state.extensions,
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(),
                        "extensions.json 损坏；从空注册面起步（包目录仍在，可重装恢复）");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Arc::new(Self {
            data_root,
            records: Mutex::new(extensions),
        })
    }

    fn persist_path(&self) -> PathBuf {
        self.data_root.join("extensions.json")
    }

    /// 包目录：`data_root/extensions/<digest>`。digest 已校验为 64 位 hex。
    pub fn package_dir(&self, digest: &str) -> PathBuf {
        self.data_root.join("extensions").join(digest)
    }

    /// 全部记录快照（按安装时间升序）。
    pub fn list(&self) -> Vec<ExtensionRecord> {
        let records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = records.clone();
        out.sort_by_key(|r| r.installed_at);
        out
    }

    pub fn get(&self, id: &str) -> Option<ExtensionRecord> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    /// 已启用扩展的记录列表（catalog 组装用）。
    pub fn enabled(&self) -> Vec<ExtensionRecord> {
        self.list().into_iter().filter(|r| r.enabled).collect()
    }

    /// 按摘要查找记录（静态服务时取期望文件摘要用）。
    pub fn find_by_digest(&self, digest: &str) -> Option<ExtensionRecord> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|r| r.digest == digest)
            .cloned()
    }

    /// 安装（或按 type 替换）一个扩展包。
    ///
    /// 流程：全量校验 → 逐文件摘要比对（digest-pinned）→ 写包目录 →
    /// 锁内替换记录 → 落盘 extensions.json → 清理被替换的孤儿包目录。
    pub fn install(&self, request: InstallRequest) -> Result<ExtensionRecord, ValidationError> {
        validate_manifest(&request.manifest)?;
        let slot = request
            .slot
            .clone()
            .unwrap_or_else(|| "workbench.grid".to_string());
        if !DEFAULT_SLOT_IDS.contains(&slot.as_str()) {
            return Err(ValidationError::new(
                "invalid_slot",
                format!("slot must be one of the known slot ids: {slot}"),
            ));
        }
        let files = validate_and_decode_files(&request.files)?;

        // 包级摘要：内容寻址目录名。排序保证同一内容集合摘要稳定。
        let mut canonical: Vec<String> = files
            .iter()
            .map(|(path, _bytes, sha)| format!("{path}:{sha}"))
            .collect();
        canonical.sort();
        let digest = sha256_hex(canonical.join("\n").as_bytes());

        // 写包目录（锁外 I/O）。同 digest 目录已存在 = 同内容，复用。
        let package_dir = self.package_dir(&digest);
        if !package_dir.exists() {
            std::fs::create_dir_all(&package_dir).map_err(|e| {
                ValidationError::new(
                    "storage_error",
                    format!("failed to create package dir: {e}"),
                )
            })?;
            for (path, bytes, _sha) in &files {
                let target = package_dir.join(path);
                std::fs::write(&target, bytes).map_err(|e| {
                    ValidationError::new("storage_error", format!("failed to write file: {e}"))
                })?;
            }
        }

        let manifest = WidgetManifest {
            // R0 硬门禁：跨源加载路径不存在——source 一律指向同源 digest 目录。
            entry: WidgetEntry {
                kind: "esm".to_string(),
                source: Some(format!("/extensions/{digest}/index.js")),
                sandbox: true,
            },
            ..request.manifest
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let record = ExtensionRecord {
            id: format!("ext-{}", uuid::Uuid::new_v4().simple()),
            widget_type: manifest.widget_type.clone(),
            digest: digest.clone(),
            installed_at: now,
            enabled: true,
            slot,
            manifest,
            files: files
                .iter()
                .map(|(path, _bytes, sha)| PackageFileMeta {
                    path: path.clone(),
                    sha256: sha.clone(),
                })
                .collect(),
        };

        // 同一 type 至多一条记录：替换旧的，随后清理孤儿包目录。
        let (replaced, snapshot) = {
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            let replaced = records
                .iter()
                .find(|r| r.widget_type == record.widget_type)
                .cloned();
            if let Some(old) = &replaced {
                records.retain(|r| r.id != old.id);
            }
            records.push(record.clone());
            (replaced, records.clone())
        };
        self.persist(&snapshot)?;

        if let Some(old) = replaced {
            if old.digest != digest && self.find_by_digest(&old.digest).is_none() {
                let _ = std::fs::remove_dir_all(self.package_dir(&old.digest));
            }
        }
        Ok(record)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Option<ExtensionRecord> {
        let snapshot = {
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            let target = records.iter_mut().find(|r| r.id == id)?;
            target.enabled = enabled;
            let updated = target.clone();
            let snapshot = records.clone();
            (snapshot, updated)
        };
        // persist 失败时回滚内存态，保证内存与盘上同真。
        if let Err(error) = self.persist(&snapshot.0) {
            tracing::error!(%error, "extensions.json 持久化失败；回滚 enable/disable");
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(target) = records.iter_mut().find(|r| r.id == id) {
                target.enabled = !enabled;
            }
            return None;
        }
        Some(snapshot.1)
    }

    /// 删除记录；若包目录不再被任何记录引用，一并清理（内容寻址可共享）。
    pub fn remove(&self, id: &str) -> Option<ExtensionRecord> {
        let (removed, snapshot) = {
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            let index = records.iter().position(|r| r.id == id)?;
            let removed = records.remove(index);
            (removed, records.clone())
        };
        if let Err(error) = self.persist(&snapshot) {
            tracing::error!(%error, "extensions.json 持久化失败；回滚删除");
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            records.push(removed.clone());
            return None;
        }
        if self.find_by_digest(&removed.digest).is_none() {
            let _ = std::fs::remove_dir_all(self.package_dir(&removed.digest));
        }
        Some(removed)
    }

    fn persist(&self, records: &[ExtensionRecord]) -> Result<(), ValidationError> {
        let state = PersistedState {
            extensions: records.to_vec(),
        };
        let json = serde_json::to_vec_pretty(&state)
            .map_err(|e| ValidationError::new("storage_error", format!("serialize: {e}")))?;
        atomic_write(&self.persist_path(), &json)
            .map_err(|e| ValidationError::new("storage_error", format!("persist: {e}")))?;
        Ok(())
    }
}

/// tmp + rename 原子写（同盘 rename；extensions.json 与 tmp 同在 data_root）。
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn is_valid_digest(segment: &str) -> bool {
    segment.len() == 64 && segment.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 内置 slot 计划已知的 slot id（与内置默认 catalog 的 slots 一致）。
pub const DEFAULT_SLOT_IDS: &[&str] = &[
    "chat.sidebar",
    "chat.panel-right",
    "settings.context",
    "diagnostics.context",
    "workbench.grid",
];

/// widget type 白名单：`ns.name` 两段，小写字母数字与 `.-_`，禁路径字符。
fn validate_widget_type(widget_type: &str) -> Result<(), ValidationError> {
    if widget_type.is_empty() || widget_type.len() > 128 {
        return Err(ValidationError::new(
            "invalid_manifest",
            "widget type must be 1..=128 chars",
        ));
    }
    let ok = widget_type
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_'))
        && widget_type.contains('.')
        && !widget_type.starts_with('.')
        && !widget_type.ends_with('.');
    if !ok {
        return Err(ValidationError::new(
            "invalid_manifest",
            format!("widget type must look like 'ns.name' (lowercase alnum/._-): {widget_type}"),
        ));
    }
    Ok(())
}

/// manifest 校验（安装面 fail-closed 三道：type/version/entry.sandbox）。
pub fn validate_manifest(manifest: &WidgetManifest) -> Result<(), ValidationError> {
    validate_widget_type(&manifest.widget_type)?;
    if manifest.version.is_empty() || manifest.version.len() > 64 {
        return Err(ValidationError::new(
            "invalid_manifest",
            "version must be 1..=64 chars",
        ));
    }
    if manifest.entry.kind != "esm" {
        return Err(ValidationError::new(
            "invalid_manifest",
            "installed extension entry.kind must be 'esm' (builtin is first-party only)",
        ));
    }
    // BUG-6 在安装面的第三道强制：缺 sandbox:true 直接拒绝，不给「授权进程内
    // 加载」的假选择（与 widget-host render 层、registry 注册面同语义）。
    if !manifest.entry.sandbox {
        return Err(ValidationError::new(
            "sandbox_required",
            "third-party esm extension must declare entry.sandbox == true (BUG-6)",
        ));
    }
    for cap in &manifest.capabilities {
        if cap.is_empty() || cap.len() > 64 || !cap.contains(':') {
            return Err(ValidationError::new(
                "invalid_manifest",
                format!("capability must look like 'scope:action': {cap}"),
            ));
        }
    }
    Ok(())
}

/// 文件清单校验 + base64 解码 + 摘要比对（digest-pinned 的安装时校验）。
/// 返回 `(path, bytes, sha256)` 三元组。
fn validate_and_decode_files(
    files: &[InstallFilePayload],
) -> Result<Vec<(String, Vec<u8>, String)>, ValidationError> {
    if files.is_empty() || files.len() > MAX_PACKAGE_FILES {
        return Err(ValidationError::new(
            "invalid_package",
            format!("package must carry 1..={} files", MAX_PACKAGE_FILES),
        ));
    }
    let mut seen = BTreeMap::new();
    let mut total = 0usize;
    for file in files {
        let path = &file.path;
        // 路径卫生：相对 POSIX 路径，无 .. / 绝对 / 反斜杠 / 空段。
        let clean = !path.is_empty()
            && !path.starts_with('/')
            && !path.contains('\\')
            && !path
                .split('/')
                .any(|seg| seg.is_empty() || seg == "." || seg == "..");
        if !clean {
            return Err(ValidationError::new(
                "invalid_package",
                format!("unsafe file path: {path}"),
            ));
        }
        if seen.contains_key(path) {
            return Err(ValidationError::new(
                "invalid_package",
                format!("duplicate file path: {path}"),
            ));
        }
        let bytes = base64_decode(&file.content_base64)
            .map_err(|e| ValidationError::new("invalid_package", format!("file {path}: {e}")))?;
        if bytes.is_empty() || bytes.len() > MAX_FILE_BYTES {
            return Err(ValidationError::new(
                "invalid_package",
                format!("file {path} exceeds size bounds"),
            ));
        }
        total += bytes.len();
        if total > MAX_PACKAGE_BYTES {
            return Err(ValidationError::new(
                "invalid_package",
                "package exceeds total size bound",
            ));
        }
        let actual = sha256_hex(&bytes);
        if !file.sha256.eq_ignore_ascii_case(&actual) {
            return Err(ValidationError::new(
                "digest_mismatch",
                format!("file {path}: declared sha256 does not match content"),
            ));
        }
        seen.insert(path.clone(), (path.clone(), bytes, actual));
    }
    if !seen.contains_key("index.js") {
        return Err(ValidationError::new(
            "invalid_package",
            "package must contain index.js (entry point)",
        ));
    }
    Ok(seen.into_values().collect())
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input.trim())
        .map_err(|e| format!("invalid base64: {e}"))
}

/// digest 段 + 相对路径 → 包内安全绝对路径（拒绝越界与非法 digest）。
pub fn resolve_package_file(
    extensions_root: &Path,
    digest: &str,
    relative: &str,
) -> Option<PathBuf> {
    if !is_valid_digest(digest) {
        return None;
    }
    let base = extensions_root.join(digest);
    let candidate = base.join(relative);
    let resolved = std::fs::canonicalize(&candidate).ok()?;
    let canonical_base = std::fs::canonicalize(&base).ok()?;
    if resolved.starts_with(canonical_base) && resolved.is_file() {
        Some(resolved)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("airp-extensions-tests")
            .join(format!("{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file_payload(path: &str, content: &[u8]) -> InstallFilePayload {
        use base64::Engine;
        InstallFilePayload {
            path: path.to_string(),
            content_base64: base64::engine::general_purpose::STANDARD.encode(content),
            sha256: sha256_hex(content),
        }
    }

    fn manifest(sandbox: bool) -> WidgetManifest {
        WidgetManifest {
            widget_type: "acme.demo".to_string(),
            version: "1.0.0".to_string(),
            title: Some("Demo".to_string()),
            author: None,
            capabilities: vec!["read:state".to_string()],
            entry: WidgetEntry {
                kind: "esm".to_string(),
                source: Some("https://evil.example/w.js".to_string()),
                sandbox,
            },
        }
    }

    fn request(sandbox: bool) -> InstallRequest {
        InstallRequest {
            manifest: manifest(sandbox),
            files: vec![file_payload(
                "index.js",
                b"export default () => ({ mount() {} });",
            )],
            slot: None,
        }
    }

    #[test]
    fn install_pins_digest_rewrites_source_and_persists() {
        let root = temp_root("install");
        let store = ExtensionStore::load(root.clone());
        let record = store.install(request(true)).expect("install ok");

        // entry.source 强制改写为同源 digest 路径（R0：跨源路径被消灭）。
        let source = record.manifest.entry.source.as_deref().unwrap();
        assert_eq!(source, format!("/extensions/{}/index.js", record.digest));
        assert!(record.manifest.entry.sandbox);

        // 文件落盘且内容一致。
        let on_disk = std::fs::read(store.package_dir(&record.digest).join("index.js")).unwrap();
        assert_eq!(on_disk, b"export default () => ({ mount() {} });");

        // 持久化：新 store 实例能读回。
        let reloaded = ExtensionStore::load(root.clone());
        assert_eq!(reloaded.list().len(), 1);
        assert_eq!(reloaded.list()[0].id, record.id);

        // 同内容重装 → 同 digest（内容寻址稳定）。
        let again = reloaded.install(request(true)).expect("reinstall ok");
        assert_eq!(again.digest, record.digest);
        assert_eq!(
            reloaded.list().len(),
            1,
            "same type replaces, never duplicates"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_rejects_missing_sandbox_flag() {
        let root = temp_root("sandbox");
        let store = ExtensionStore::load(root.clone());
        let error = store.install(request(false)).unwrap_err();
        assert_eq!(error.code, "sandbox_required");
        assert!(store.list().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_validates_slot_and_defaults() {
        let root = temp_root("slot");
        let store = ExtensionStore::load(root.clone());

        // 未知 slot → invalid_slot（fail-closed，不默默编入任意位置）。
        let mut bad = request(true);
        bad.slot = Some("not.a.slot".to_string());
        assert_eq!(store.install(bad).unwrap_err().code, "invalid_slot");

        // 缺省 → workbench.grid；显式合法 slot 被保留。
        let record = store.install(request(true)).expect("default slot ok");
        assert_eq!(record.slot, "workbench.grid");
        let mut explicit = request(true);
        explicit.manifest.widget_type = "acme.demo2".to_string();
        explicit.slot = Some("chat.sidebar".to_string());
        let record = store.install(explicit).expect("explicit slot ok");
        assert_eq!(record.slot, "chat.sidebar");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_rejects_digest_mismatch() {
        let root = temp_root("digest");
        let store = ExtensionStore::load(root.clone());
        let mut req = request(true);
        req.files[0].sha256 = "0".repeat(64);
        let error = store.install(req).unwrap_err();
        assert_eq!(error.code, "digest_mismatch");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_rejects_unsafe_paths_and_missing_entry() {
        let root = temp_root("paths");
        let store = ExtensionStore::load(root.clone());
        for bad in ["../evil.js", "/abs.js", "a\\b.js", "a//b.js"] {
            let req = InstallRequest {
                manifest: manifest(true),
                files: vec![file_payload(bad, b"x")],
                slot: None,
            };
            assert_eq!(
                store.install(req).unwrap_err().code,
                "invalid_package",
                "{bad}"
            );
        }
        let req = InstallRequest {
            manifest: manifest(true),
            files: vec![file_payload("main.js", b"x")],
            slot: None,
        };
        let error = store.install(req).unwrap_err();
        assert_eq!(error.code, "invalid_package");
        assert!(error.message.contains("index.js"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enable_disable_delete_lifecycle() {
        let root = temp_root("lifecycle");
        let store = ExtensionStore::load(root.clone());
        let record = store.install(request(true)).unwrap();

        let disabled = store.set_enabled(&record.id, false).unwrap();
        assert!(!disabled.enabled);
        assert!(
            store.enabled().is_empty(),
            "disabled extensions leave the catalog"
        );

        let enabled = store.set_enabled(&record.id, true).unwrap();
        assert!(enabled.enabled);
        assert_eq!(store.enabled().len(), 1);

        let removed = store.remove(&record.id).unwrap();
        assert_eq!(removed.id, record.id);
        assert!(store.list().is_empty());
        assert!(
            !store.package_dir(&record.digest).exists(),
            "orphan package dir must be cleaned"
        );
        assert!(store.remove(&record.id).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_package_file_blocks_traversal_and_bad_digest() {
        let root = temp_root("resolve");
        let store = ExtensionStore::load(root.clone());
        let record = store.install(request(true)).unwrap();
        let extensions_root = root.join("extensions");

        assert!(resolve_package_file(&extensions_root, &record.digest, "index.js").is_some());
        assert!(
            resolve_package_file(&extensions_root, &record.digest, "../extensions.json").is_none()
        );
        assert!(resolve_package_file(&extensions_root, "not-a-digest", "index.js").is_none());
        assert!(resolve_package_file(&extensions_root, &record.digest, "missing.js").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
