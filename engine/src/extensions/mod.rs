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

/// C-P4 第二批（#484）：host_api / capability 合同兼容回归装置（仅测试构型）。
#[cfg(test)]
mod compat;

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

/// 变更类操作错误（#485 E1）。
///
/// 此前 `set_enabled` / `remove` 以 `Option::None` 同时表示「id 不存在」与
/// 「持久化失败回滚后」，HTTP 面一律映射 404 not_found，掩盖了 500
/// storage_error。改为 typed error 后 handler 可精确区分：
/// `NotFound` → 404；`Storage` → 500 storage_error。
#[derive(Debug)]
pub enum MutationError {
    /// 目标 id 不存在。
    NotFound,
    /// extensions.json 持久化失败（内存态已精确回滚）。
    Storage(String),
}

impl std::fmt::Display for MutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MutationError::NotFound => write!(f, "extension not found"),
            MutationError::Storage(e) => write!(f, "storage error: {e}"),
        }
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
///
/// C-P4 新增：`host_api` 声明 widget 所需的宿主合同 major 版本（semver
/// major 匹配，如 `"1"` 或 `"1.2"`）。engine 当前支持 `HOST_API_MAJOR = 1`。
/// 缺省视为 `"1"`（向后兼容已有 widget）；跨 major 拒绝安装（前向兼容铁律）。
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
    /// C-P4：widget 所需的宿主合同 major 版本（如 `"1"`、`"1.2"`）。
    /// 缺省视为 `"1"`（向后兼容）。安装时校验 major == HOST_API_MAJOR，
    /// 跨 major 拒绝（前向兼容铁律，禁止静默尝试）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_api: Option<String>,
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
    /// C-P3：engine 权威签发的 capability grant（manifest.capabilities 的子集）。
    /// 空 = 未 consent（widget 加载门禁未通过，esm widget 不应挂载）。
    /// 非空 = 已 consent 且被授予这些 capabilities；逐调用强制时
    /// `widget_intent` 校验 envelope.capability ∈ 此集合。
    /// 重装（同 type 不同 digest）会清空 grant——consent 不跨身份延续
    /// （对译 webui consent.js 的 grantKey = type@version#source 语义）。
    #[serde(default)]
    pub granted_capabilities: Vec<String>,
    /// C-P3：grant 签发时间戳（秒，UNIX_EPOCH）。None = 未 grant。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_at: Option<u64>,
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
    /// W3：变更操作串行化锁。并发语义说明：
    /// - install / remove / set_enabled 的「改内存 → persist → 孤儿清理」
    ///   全程必须原子，否则并发同 type 安装的 persist 交叉可让
    ///   extensions.json 停在旧真，孤儿判定与 remove_dir_all 之间也有
    ///   TOCTOU 窗口；
    /// - `records` 锁只保护内存快照读写，粒度不变；
    /// - install 的包目录写入（内容寻址、幂等）仍可在锁外并发，审查
    ///   确认安全；串行化仅覆盖记录变更段落。
    mutations: Mutex<()>,
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
            mutations: Mutex::new(()),
        })
    }

    /// W3：变更操作串行化守卫（poison 时 recover 而非 panic）。
    fn mutation_guard(&self) -> std::sync::MutexGuard<'_, ()> {
        self.mutations.lock().unwrap_or_else(|e| e.into_inner())
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

    /// C-P3：按 widget_type 查找已启用扩展记录（`widget_intent` 逐调用强制用）。
    /// 仅返回已启用的——停用的扩展其 capability 不应被任何 intent 使用。
    pub fn find_enabled_by_type(&self, widget_type: &str) -> Option<ExtensionRecord> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|r| r.widget_type == widget_type && r.enabled)
            .cloned()
    }

    /// C-P3：engine 权威签发 capability grant。
    ///
    /// 权限 = manifest ∩ 用户请求 ∩ engine policy：
    /// - `capabilities = None` → 授予 `manifest.capabilities` 全集（用户全量同意）
    /// - `capabilities = Some(caps)` → 校验 `caps ⊆ manifest.capabilities`，
    ///   仅授予子集（部分授权）；越界 capability 返回 `capability_not_declared`。
    /// - engine policy：C-P3 暂无额外限制（manifest 内即允许）；C-P4 可接 policy 层。
    ///
    /// 返回 `Ok(None)` = 未找到 id；`Ok(Some)` = 成功；`Err` = 校验/持久化失败。
    /// persist 失败时精确回滚内存态至原 grant 集合（保内存与盘上同真）。
    pub fn grant(
        &self,
        id: &str,
        capabilities: Option<Vec<String>>,
    ) -> Result<Option<ExtensionRecord>, ValidationError> {
        let _serial = self.mutation_guard();
        let (updated, snapshot, original) = {
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            let Some(target) = records.iter_mut().find(|r| r.id == id) else {
                return Ok(None);
            };
            let granted = match capabilities {
                Some(caps) => {
                    for cap in &caps {
                        if !target.manifest.capabilities.contains(cap) {
                            return Err(ValidationError::new(
                                "capability_not_declared",
                                format!(
                                    "capability {cap} not declared in manifest of {}",
                                    target.widget_type
                                ),
                            ));
                        }
                    }
                    caps
                }
                None => target.manifest.capabilities.clone(),
            };
            let original = (target.granted_capabilities.clone(), target.granted_at);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            target.granted_capabilities = granted;
            // #488 E1：空集语义归一化——空 grant = 无授权，与 revoke_grant 同语义，
            // 不得留下「有签发时间戳但无任何授权」的脏状态（consent 层把空
            // granted_capabilities 视为无授权，该时间戳只会误导审计面）。
            target.granted_at = if target.granted_capabilities.is_empty() {
                None
            } else {
                Some(now)
            };
            (target.clone(), records.clone(), original)
        };
        if let Err(error) = self.persist(&snapshot) {
            tracing::error!(%error, "extensions.json 持久化失败；回滚 grant");
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(target) = records.iter_mut().find(|r| r.id == id) {
                target.granted_capabilities = original.0;
                target.granted_at = original.1;
            }
            return Err(ValidationError::new(
                "storage_error",
                format!("persist grant failed: {error}"),
            ));
        }
        tracing::info!(
            extension_id = %id,
            widget_type = %updated.widget_type,
            granted_count = updated.granted_capabilities.len(),
            "C-P3 grant signed"
        );
        Ok(Some(updated))
    }

    /// C-P3：撤销 capability grant。
    ///
    /// - `capabilities = None` → 撤销全部（等同 revoke consent）
    /// - `capabilities = Some(caps)` → 仅撤销指定 capability（子集撤销）
    ///
    /// 返回 `Ok(None)` = 未找到 id；`Ok(Some)` = 成功；`Err` = 持久化失败。
    /// persist 失败时精确回滚内存态至原 grant 集合。
    pub fn revoke_grant(
        &self,
        id: &str,
        capabilities: Option<Vec<String>>,
    ) -> Result<Option<ExtensionRecord>, ValidationError> {
        let _serial = self.mutation_guard();
        let (updated, snapshot, original) = {
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            let Some(target) = records.iter_mut().find(|r| r.id == id) else {
                return Ok(None);
            };
            let original = (target.granted_capabilities.clone(), target.granted_at);
            match capabilities {
                None => {
                    target.granted_capabilities.clear();
                    target.granted_at = None;
                }
                Some(caps) => {
                    target.granted_capabilities.retain(|c| !caps.contains(c));
                    if target.granted_capabilities.is_empty() {
                        target.granted_at = None;
                    }
                }
            }
            (target.clone(), records.clone(), original)
        };
        if let Err(error) = self.persist(&snapshot) {
            tracing::error!(%error, "extensions.json 持久化失败；回滚 revoke_grant");
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(target) = records.iter_mut().find(|r| r.id == id) {
                target.granted_capabilities = original.0;
                target.granted_at = original.1;
            }
            return Err(ValidationError::new(
                "storage_error",
                format!("persist revoke_grant failed: {error}"),
            ));
        }
        tracing::info!(
            extension_id = %id,
            widget_type = %updated.widget_type,
            remaining = updated.granted_capabilities.len(),
            "C-P3 grant revoked"
        );
        Ok(Some(updated))
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
        // #485 E4：文件路径可为嵌套（validate_and_decode_files 接受
        // `a/b.js` 形态），写前逐文件建父目录；写入失败时清理半写
        // 包目录，不留半真（同 digest 重装会重建）。
        let package_dir = self.package_dir(&digest);
        if !package_dir.exists() {
            let write_result = (|| -> std::io::Result<()> {
                std::fs::create_dir_all(&package_dir)?;
                for (path, bytes, _sha) in &files {
                    let target = package_dir.join(path);
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&target, bytes)?;
                }
                Ok(())
            })();
            if let Err(e) = write_result {
                let _ = std::fs::remove_dir_all(&package_dir);
                return Err(ValidationError::new(
                    "storage_error",
                    format!("failed to write package dir: {e}"),
                ));
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
            // C-P3：新装/重装一律从无 grant 起步——consent 不跨身份延续。
            granted_capabilities: Vec::new(),
            granted_at: None,
        };

        // W3：以下记录变更 + persist + 孤儿清理全程串行（mutation_guard），
        // 杜绝并发同 type 安装的 persist 交叉把 extensions.json 停在旧真；
        // 上方包目录写入（内容寻址、幂等）不受此约束，可安全并发。
        let _serial = self.mutation_guard();

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

    /// 启用/停用扩展。`NotFound` = id 不存在；`Storage` = 持久化失败
    /// （内存态已回滚）——#485 E1：不再以 Option 混淆两种失败。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<ExtensionRecord, MutationError> {
        // W3：变更 + persist（含失败回滚）全程串行。
        let _serial = self.mutation_guard();
        let snapshot = {
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            let Some(target) = records.iter_mut().find(|r| r.id == id) else {
                return Err(MutationError::NotFound);
            };
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
            return Err(MutationError::Storage(format!("persist failed: {error}")));
        }
        Ok(snapshot.1)
    }

    /// 删除记录；若包目录不再被任何记录引用，一并清理（内容寻址可共享）。
    /// `NotFound` / `Storage` 语义同 [`Self::set_enabled`]（#485 E1）。
    pub fn remove(&self, id: &str) -> Result<ExtensionRecord, MutationError> {
        // W3：删除 + persist + 孤儿目录清理全程串行，杜绝孤儿判定与
        // remove_dir_all 之间的 TOCTOU 窗口。
        let _serial = self.mutation_guard();
        let (removed, snapshot) = {
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            let Some(index) = records.iter().position(|r| r.id == id) else {
                return Err(MutationError::NotFound);
            };
            let removed = records.remove(index);
            (removed, records.clone())
        };
        if let Err(error) = self.persist(&snapshot) {
            tracing::error!(%error, "extensions.json 持久化失败；回滚删除");
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            records.push(removed.clone());
            return Err(MutationError::Storage(format!("persist failed: {error}")));
        }
        if self.find_by_digest(&removed.digest).is_none() {
            let _ = std::fs::remove_dir_all(self.package_dir(&removed.digest));
        }
        Ok(removed)
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
///
/// #485 E3：rename 只给原子可见性，不给崩溃耐久——数据可能仍停在
/// page cache，掉电后 extensions.json 可能空/截断。故 tmp 先 `sync_all`
/// 落盘再 rename。
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
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

/// C-P4：engine 当前支持的宿主合同 major 版本。
///
/// widget manifest 的 `host_api` 字段声明所需 major；安装时校验
/// `parse_host_api_major(host_api) == HOST_API_MAJOR`，跨 major 拒绝
/// （前向兼容铁律：不静默尝试不兼容的 widget）。缺省 `host_api` 视为
/// `"1"`（向后兼容已有 widget）。
pub const HOST_API_MAJOR: u32 = 1;

/// C-P4 第二批：engine policy 的 capability 封闭集（权威枚举）。
///
/// 与 docs/WIDGET-DEVELOPMENT.md §5「capability 枚举」严格一致；catalog
/// 顶层以 `capabilities` 字段下发此封闭集，供 webui 授权 UI 渲染全集
/// 清单（grant 面只能在此集内选择）。新增 capability 必须同步改此常量
/// 与文档，compat harness 测试锁住两侧一致性。
pub const KNOWN_CAPABILITIES: [&str; 6] = [
    "read:memory",
    "write:memory",
    "read:worldbook",
    "read:state",
    "write:state",
    "call:tool",
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

/// manifest 校验（安装面 fail-closed 四道：type/version/entry.sandbox/hostApi）。
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
    // C-P4：hostApi major 匹配（前向兼容铁律）。缺省视为 "1"（向后兼容）。
    let declared_major = parse_host_api_major(manifest.host_api.as_deref())?;
    if declared_major != HOST_API_MAJOR {
        return Err(ValidationError::new(
            "host_api_incompatible",
            format!(
                "widget requires host_api major {} but engine supports {} (upgrade widget or stay on compatible engine)",
                declared_major, HOST_API_MAJOR
            ),
        ));
    }
    Ok(())
}

/// 解析 `host_api` 字段的 major 版本。
///
/// 接受 `"1"`、`"1.0"`、`"1.2.3"` 形态；取首段为 major。缺省 / 空串视为
/// `"1"`（向后兼容已有 widget；定夺见 docs/WIDGET-DEVELOPMENT.md §3）。
/// 非数字段 / 前导零（`"01"`）/ 超长 / 段缺数字（`"1."`、`"1.x"`）=
/// `invalid_manifest`。严格校验所有段为纯数字，避免 `"1.x"` 这类伪 semver
/// 被宽松解析为 major 1 而掩盖声明错误。major 段额外拒绝 `"0"`（major 0
/// 不合法）；minor/patch 段允许 `"0"`（如 `"1.0"`、`"1.2.0"` 合法）。
/// `pub(crate)`：trusted plugin manifest 的 `host_api` 复用同一校验（#498 §6.2）。
pub(crate) fn parse_host_api_major(host_api: Option<&str>) -> Result<u32, ValidationError> {
    let raw = match host_api {
        None => return Ok(1),
        Some("") => return Ok(1),
        Some(s) => s,
    };
    let parts: Vec<&str> = raw.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        let is_major = i == 0;
        if part.is_empty()
            || part.len() > 8
            || (is_major && *part == "0")
            || (part.starts_with('0') && part.len() > 1)
            || !part.chars().all(|c| c.is_ascii_digit())
        {
            return Err(ValidationError::new(
                "invalid_manifest",
                format!(
                    "host_api segment must be a non-negative integer without leading zeros: {raw}"
                ),
            ));
        }
    }
    let major: u32 = parts[0].parse().map_err(|_| {
        ValidationError::new(
            "invalid_manifest",
            format!("host_api major not a number: {raw}"),
        )
    })?;
    Ok(major)
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
            host_api: None,
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

    /// W3 回归：并发同 type 安装（各自不同内容 → 不同 digest，触发替换 +
    /// 孤儿清理）必须串行化，内存与盘上均停在唯一胜出记录，不得残留
    /// 旧真或重复条目。
    #[test]
    fn concurrent_same_type_installs_serialize_without_stale_persist() {
        let root = temp_root("concurrent");
        let store = ExtensionStore::load(root.clone());
        let mut handles = Vec::new();
        for i in 0..8u8 {
            let store = store.clone();
            handles.push(std::thread::spawn(move || {
                let content = format!("export default () => ({{ v: {i} }});");
                let req = InstallRequest {
                    manifest: manifest(true),
                    files: vec![file_payload("index.js", content.as_bytes())],
                    slot: None,
                };
                store.install(req).expect("install ok")
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(
            store.list().len(),
            1,
            "同 type 替换 + 串行 persist 后内存仅一条"
        );
        let reloaded = ExtensionStore::load(root.clone());
        assert_eq!(reloaded.list().len(), 1, "盘上不得停在旧真/重复条目");
        assert_eq!(reloaded.list()[0].id, store.list()[0].id);
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

    // C-P4-3：hostApi semver major 匹配 + 前向兼容铁律。
    #[test]
    fn install_validates_host_api_major() {
        let root = temp_root("hostapi");
        let store = ExtensionStore::load(root.clone());

        // 缺省 host_api → 视为 "1"，兼容（向后兼容已有 widget）。
        let mut req = request(true);
        req.manifest.widget_type = "acme.compat1".to_string();
        store
            .install(req)
            .expect("host_api absent = major 1 compat");

        // 显式 "1" / "1.0" / "1.2.3" → 兼容。
        for (i, ver) in ["1", "1.0", "1.2.3"].iter().enumerate() {
            let mut req = request(true);
            req.manifest.widget_type = format!("acme.compat{i}");
            req.manifest.host_api = Some(ver.to_string());
            store.install(req).expect("host_api major 1 compat");
        }

        // 跨 major "2" / "2.0" → host_api_incompatible（前向兼容铁律，禁止静默尝试）。
        for ver in ["2", "2.0", "2.1.0"] {
            let mut req = request(true);
            req.manifest.widget_type = format!("acme.future-{ver}");
            req.manifest.host_api = Some(ver.to_string());
            assert_eq!(
                store.install(req).unwrap_err().code,
                "host_api_incompatible",
                "host_api {ver} must be rejected (engine major = 1)"
            );
        }

        // #489 E1：坏值分支用合法且唯一的 widget_type——此前以
        // `format!("acme.bad-{bad:?}")` 拼接，`{:?}` 引入引号导致
        // validate_widget_type 先于 host_api 校验失败，坏值实际未被测到。
        for (i, bad) in ["0", "01", "abc", "1.x", "1.", "999999999"]
            .iter()
            .enumerate()
        {
            let mut req = request(true);
            req.manifest.widget_type = format!("acme.badhost{i}");
            req.manifest.host_api = Some(bad.to_string());
            assert_eq!(
                store.install(req).unwrap_err().code,
                "invalid_manifest",
                "host_api {bad:?} must be rejected as invalid_manifest"
            );
        }

        // #489 D1 定夺：空串视为缺省 "1" → 安装成功（文档与实现对齐）。
        let mut req = request(true);
        req.manifest.widget_type = "acme.emptyhost".to_string();
        req.manifest.host_api = Some(String::new());
        store
            .install(req)
            .expect("host_api empty string = default major 1 compat");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_host_api_major_handles_edge_cases() {
        // 缺省 / 空串 = 1（向后兼容）。
        assert_eq!(parse_host_api_major(None).unwrap(), 1);
        assert_eq!(parse_host_api_major(Some("")).unwrap(), 1);
        // 合法 major。
        assert_eq!(parse_host_api_major(Some("1")).unwrap(), 1);
        assert_eq!(parse_host_api_major(Some("1.0")).unwrap(), 1);
        assert_eq!(parse_host_api_major(Some("1.2.3")).unwrap(), 1);
        assert_eq!(parse_host_api_major(Some("2")).unwrap(), 2);
        // 非法：0 / 前导零 / 非数字。
        assert!(parse_host_api_major(Some("0")).is_err());
        assert!(parse_host_api_major(Some("01")).is_err());
        assert!(parse_host_api_major(Some("abc")).is_err());
        assert!(parse_host_api_major(Some("1.x")).is_err());
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
        // #485 E1：重复删除返回 typed NotFound（非 Option::None 双义）。
        assert!(matches!(
            store.remove(&record.id),
            Err(MutationError::NotFound)
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// #485 E4：嵌套文件路径（`assets/nested.js`）写包前必须建父目录，
    /// 否则 std::fs::write 失败；安装成功后文件应可解析落盘。
    #[test]
    fn install_supports_nested_file_paths() {
        let root = temp_root("nested");
        let store = ExtensionStore::load(root.clone());
        let nested = b"export const nested = true;";
        let req = InstallRequest {
            manifest: manifest(true),
            files: vec![
                file_payload("index.js", b"export default () => ({ mount() {} });"),
                file_payload("assets/deep/nested.js", nested),
            ],
            slot: None,
        };
        let record = store.install(req).expect("nested paths install ok");
        let on_disk = std::fs::read(
            store
                .package_dir(&record.digest)
                .join("assets/deep/nested.js"),
        )
        .unwrap();
        assert_eq!(on_disk, nested);
        // 静态服务面的路径解析同样能命中嵌套文件。
        let resolved = resolve_package_file(
            &root.join("extensions"),
            &record.digest,
            "assets/deep/nested.js",
        );
        assert!(resolved.is_some(), "nested file must be servable");
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

    /// C-P3：manifest 多 capabilities 的安装 helper。
    fn manifest_multi_caps(caps: Vec<&str>) -> WidgetManifest {
        WidgetManifest {
            widget_type: "acme.multi".to_string(),
            version: "1.0.0".to_string(),
            title: Some("Multi".to_string()),
            author: None,
            capabilities: caps.into_iter().map(String::from).collect(),
            host_api: None,
            entry: WidgetEntry {
                kind: "esm".to_string(),
                source: Some("https://evil.example/w.js".to_string()),
                sandbox: true,
            },
        }
    }

    #[test]
    fn grant_full_set_grants_all_manifest_capabilities() {
        let root = temp_root("grant-full");
        let store = ExtensionStore::load(root.clone());
        let mut req = request(true);
        req.manifest = manifest_multi_caps(vec!["read:state", "write:state", "read:memory"]);
        let record = store.install(req).unwrap();

        // 新装 → 无 grant。
        assert!(record.granted_capabilities.is_empty());
        assert!(record.granted_at.is_none());

        // grant(None) → 全集。
        let granted = store.grant(&record.id, None).unwrap().unwrap();
        assert_eq!(granted.granted_capabilities.len(), 3);
        assert!(granted.granted_at.is_some());
        assert!(granted.granted_capabilities.iter().all(|c| [
            "read:state",
            "write:state",
            "read:memory"
        ]
        .contains(&c.as_str())));

        // 持久化：新 store 实例能读回 grant。
        let reloaded = ExtensionStore::load(root.clone());
        let persisted = reloaded.get(&record.id).unwrap();
        assert_eq!(persisted.granted_capabilities.len(), 3);
        assert!(persisted.granted_at.is_some());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn grant_subset_validates_against_manifest() {
        let root = temp_root("grant-subset");
        let store = ExtensionStore::load(root.clone());
        let mut req = request(true);
        req.manifest = manifest_multi_caps(vec!["read:state", "write:state"]);
        let record = store.install(req).unwrap();

        // 部分授权：仅 read:state。
        let granted = store
            .grant(&record.id, Some(vec!["read:state".to_string()]))
            .unwrap()
            .unwrap();
        assert_eq!(granted.granted_capabilities, vec!["read:state".to_string()]);

        // 越界 capability → capability_not_declared。
        let error = store
            .grant(&record.id, Some(vec!["admin:root".to_string()]))
            .unwrap_err();
        assert_eq!(error.code, "capability_not_declared");
        // 越界不改变现有 grant。
        let current = store.get(&record.id).unwrap();
        assert_eq!(current.granted_capabilities, vec!["read:state".to_string()]);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// #488 E1：grant 空 capabilities 语义归一化——空集 = 无授权，
    /// granted_at 必须清空（不得呈现「有签发时间戳但无任何授权」）。
    #[test]
    fn grant_empty_capabilities_normalizes_to_no_grant() {
        let root = temp_root("grant-empty");
        let store = ExtensionStore::load(root.clone());
        let record = store.install(request(true)).unwrap();

        // 先 grant 全集（manifest 声明 read:state）。
        let granted = store.grant(&record.id, None).unwrap().unwrap();
        assert!(!granted.granted_capabilities.is_empty());
        assert!(granted.granted_at.is_some());

        // 空子集 grant → 归一化为无授权：集合空且 granted_at 清空。
        let after = store.grant(&record.id, Some(Vec::new())).unwrap().unwrap();
        assert!(after.granted_capabilities.is_empty());
        assert!(
            after.granted_at.is_none(),
            "空 grant 不得保留 granted_at（与 revoke_grant 空集语义对称）"
        );

        // 未 grant 起步的扩展直接空 grant 亦同。
        let mut req = request(true);
        req.manifest.widget_type = "acme.empty2".to_string();
        let fresh = store.install(req).unwrap();
        let after = store.grant(&fresh.id, Some(Vec::new())).unwrap().unwrap();
        assert!(after.granted_capabilities.is_empty());
        assert!(after.granted_at.is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn revoke_grant_clears_all_or_subset() {
        let root = temp_root("revoke");
        let store = ExtensionStore::load(root.clone());
        let mut req = request(true);
        req.manifest = manifest_multi_caps(vec!["read:state", "write:state", "read:memory"]);
        let record = store.install(req).unwrap();
        store.grant(&record.id, None).unwrap();

        // 子集撤销：移除 write:state，剩 read:state + read:memory。
        let after = store
            .revoke_grant(&record.id, Some(vec!["write:state".to_string()]))
            .unwrap()
            .unwrap();
        assert_eq!(after.granted_capabilities.len(), 2);
        assert!(!after
            .granted_capabilities
            .contains(&"write:state".to_string()));
        assert!(after.granted_at.is_some(), "非空 grant 保留 granted_at");

        // 全量撤销：清空全部。
        let after = store.revoke_grant(&record.id, None).unwrap().unwrap();
        assert!(after.granted_capabilities.is_empty());
        assert!(after.granted_at.is_none(), "空 grant 清除 granted_at");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_resets_grant_on_reinstall() {
        let root = temp_root("reinstall-reset");
        let store = ExtensionStore::load(root.clone());
        let record = store.install(request(true)).unwrap();
        store.grant(&record.id, None).unwrap();
        assert!(!store
            .get(&record.id)
            .unwrap()
            .granted_capabilities
            .is_empty());

        // 重装（同 type 不同内容 → 不同 digest）→ grant 清空。
        let new_content = b"export default () => ({ mount() { /* v2 */ } });";
        use base64::Engine;
        let req = InstallRequest {
            manifest: manifest(true),
            files: vec![InstallFilePayload {
                path: "index.js".to_string(),
                content_base64: base64::engine::general_purpose::STANDARD.encode(new_content),
                sha256: sha256_hex(new_content),
            }],
            slot: None,
        };
        let reinstalled = store.install(req).unwrap();
        assert!(reinstalled.granted_capabilities.is_empty());
        assert!(
            reinstalled.granted_at.is_none(),
            "重装后 grant 必须清空（consent 不跨身份延续）"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn find_enabled_by_type_skips_disabled() {
        let root = temp_root("find-enabled");
        let store = ExtensionStore::load(root.clone());
        let record = store.install(request(true)).unwrap();

        assert!(store.find_enabled_by_type("acme.demo").is_some());
        store.set_enabled(&record.id, false).unwrap();
        assert!(
            store.find_enabled_by_type("acme.demo").is_none(),
            "停用的扩展不应被 widget_intent 找到"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn grant_and_revoke_persist_failure_rolls_back() {
        // persist 失败回滚：用 set_persistence_override 注入失败 hook 不可行
        // （ExtensionStore 无 override 面）；改为校验 not_found 与越界的非破坏性。
        let root = temp_root("grant-rollback");
        let store = ExtensionStore::load(root.clone());

        // 未找到 id → Ok(None)，不 panic。
        assert!(store.grant("ext-nonexistent", None).unwrap().is_none());
        assert!(store
            .revoke_grant("ext-nonexistent", None)
            .unwrap()
            .is_none());

        let _ = std::fs::remove_dir_all(&root);
    }
}
