use crate::adapter::{BackendEngine, Provider};
use crate::quota::QuotaConfig;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// M4.3：`data/settings.json` 的 partial schema。所有字段 Option，缺省即沿用上一层。
#[derive(Debug, Deserialize, Default)]
pub struct PartialAppConfig {
    /// 供应商标识，缺省沿用上层。
    pub provider: Option<Provider>,
    /// 完整端点 URL。
    pub endpoint: Option<String>,
    /// API key。
    pub api_key: Option<String>,
    /// 上游模型 ID。
    pub model: Option<String>,
    /// daemon 监听端口。
    pub daemon_port: Option<u16>,
    /// 卷系统参数；缺省沿用 [`VolumeConfig::default`]。
    pub volume: Option<VolumeConfig>,
    /// DX-2：daemon 访问鉴权 key（客户端须携带 `Authorization: Bearer <key>`）。
    /// 为 None/空字符串时不启用鉴权，任何请求均放行。
    pub access_api_key: Option<String>,
    /// DX-6：后端引擎选择；缺省沿用上层。
    pub engine: Option<BackendEngine>,
    /// DX-3：每日配额限制；缺省沿用上层。
    pub quota: Option<QuotaConfig>,
}

/// 卷系统的运行参数。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VolumeConfig {
    /// 软压力阈值：current.md 超过此 token 数时在 System Prompt 中提示 AI 寻找自然封卷点。
    pub soft_threshold_tokens: usize,
    /// 硬阈值：current.md 超过此 token 数时强制封卷，无论 AI 是否同意。
    pub hard_threshold_tokens: usize,
    /// 封卷调用使用的 temperature，建议偏低以求结构稳定。
    pub seal_temperature: f32,
    /// 封卷使用的模型；为 None 时复用 AppConfig.model。
    pub seal_model: Option<String>,
    /// 维护触发间隔：每 N 轮对话执行一次 index 修剪。
    pub maintenance_interval: u32,
}

impl Default for VolumeConfig {
    fn default() -> Self {
        VolumeConfig {
            soft_threshold_tokens: 2500,
            hard_threshold_tokens: 3500,
            seal_temperature: 0.3,
            seal_model: None,
            maintenance_interval: 20,
        }
    }
}

impl VolumeConfig {
    /// M0 F-12 / 5.0b：验证 soft<hard 不变量。
    /// 配置合并完成后必须调用，避免运行时静默失效（soft 提示永远先于 hard 触发，
    /// 否则当 soft >= hard 时，hard 强封会先发生，soft 提示形同虚设）。
    pub fn validate(&self) -> Result<(), String> {
        if self.soft_threshold_tokens >= self.hard_threshold_tokens {
            return Err(format!(
                "VolumeConfig 不变量违反：soft_threshold_tokens ({}) 必须小于 hard_threshold_tokens ({})",
                self.soft_threshold_tokens, self.hard_threshold_tokens
            ));
        }
        if self.seal_temperature < 0.0 || self.seal_temperature > 2.0 {
            return Err(format!(
                "VolumeConfig.seal_temperature ({}) 必须在 [0.0, 2.0] 范围内",
                self.seal_temperature
            ));
        }
        if self.maintenance_interval == 0 {
            return Err("VolumeConfig.maintenance_interval 必须 > 0".to_string());
        }
        Ok(())
    }
}

/// daemon 启动期最终生效的全局配置，三层合并产物
/// （`default → data/settings.json → AIRP_* env → request body`）。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    /// 供应商标识。
    pub provider: Provider,
    /// 上游 chat/completions 端点 URL（含完整路径）。
    pub endpoint: String,
    /// 上游 API key；`None` 时不发 `Authorization` 头。
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    /// 默认模型 ID（请求体未指定时使用）。
    pub model: String,
    /// daemon 监听端口。
    pub daemon_port: u16,
    /// 卷系统运行参数。
    #[serde(default)]
    pub volume: VolumeConfig,
    /// DX-2：daemon 访问鉴权 key。设置后 `/v1/*` 端点要求 `Authorization: Bearer <key>`。
    /// 为 None 时不启用鉴权（单用户本地模式默认）。
    #[serde(default, skip_serializing)]
    pub access_api_key: Option<String>,
    /// DX-6：后端引擎；缺省 `Direct`（OpenAI compat）。
    #[serde(default)]
    pub engine: BackendEngine,
    /// DX-3：每日配额；缺省不限制。
    #[serde(default)]
    pub quota: QuotaConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            provider: Provider::OpenAI,
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            api_key: None,
            model: "gpt-4o".to_string(),
            daemon_port: 8000,
            volume: VolumeConfig::default(),
            access_api_key: None,
            engine: BackendEngine::default(),
            quota: QuotaConfig::default(),
        }
    }
}

impl AppConfig {
    /// 从指定路径的 JSON 配置文件中加载配置。
    /// 如果文件不存在，则写入默认配置并返回。
    pub fn load_or_create<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path_ref = path.as_ref();
        if path_ref.exists() {
            let content =
                fs::read_to_string(path_ref).map_err(|e| format!("无法读取配置文件: {}", e))?;
            let mut config: AppConfig =
                serde_json::from_str(&content).map_err(|e| format!("解析配置文件失败: {}", e))?;
            // Secrets are runtime-only. Legacy files may still contain these fields,
            // but loading them would silently perpetuate plaintext credential storage.
            config.api_key = None;
            config.access_api_key = None;
            Ok(config)
        } else {
            let default_config = AppConfig::default();
            let content = serde_json::to_string_pretty(&default_config)
                .map_err(|e| format!("序列化默认配置失败: {}", e))?;
            if let Some(parent) = path_ref.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("创建配置文件夹失败: {}", e))?;
            }
            fs::write(path_ref, content).map_err(|e| format!("写入默认配置文件失败: {}", e))?;
            Ok(default_config)
        }
    }

    /// M4.3 配置三层合并：第二层 = `data/settings.json`。
    ///
    /// 调用顺序：`AppConfig::default()` → `load_or_create(config.json)`
    /// → **此函数（data/settings.json）** → `override_with_env()` → 最终请求层覆盖。
    /// 空字符串视为"未设置"以避免默认模板里的 `endpoint: ""` 覆盖掉合理上层值。
    pub fn merge_data_settings(&mut self, data_root: &Path) -> Result<(), String> {
        let path = data_root.join("settings.json");
        if !path.exists() {
            return Ok(());
        }
        let raw =
            fs::read_to_string(&path).map_err(|e| format!("读取 settings.json 失败: {}", e))?;
        let partial: PartialAppConfig =
            serde_json::from_str(&raw).map_err(|e| format!("解析 settings.json 失败: {}", e))?;
        if let Some(p) = partial.provider {
            self.provider = p;
        }
        if let Some(e) = partial.endpoint.filter(|s| !s.is_empty()) {
            self.endpoint = e;
        }
        if let Some(m) = partial.model.filter(|s| !s.is_empty()) {
            self.model = m;
        }
        if let Some(port) = partial.daemon_port {
            self.daemon_port = port;
        }
        if let Some(v) = partial.volume {
            self.volume = v;
        }
        if let Some(e) = partial.engine {
            self.engine = e;
        }
        if let Some(q) = partial.quota {
            self.quota = q;
        }
        Ok(())
    }

    /// M0 F-12 / 5.0b：在所有配置层合并完成后调用，验证跨字段不变量。
    /// 启动时 fast-fail，避免运行期才发现 soft >= hard 之类的静默错误。
    pub fn validate(&self) -> Result<(), String> {
        self.volume.validate()?;
        Ok(())
    }

    /// 从环境变量读取，用来覆盖配置。
    ///
    /// 注：自统一为 OpenAI 兼容协议后，`AIRP_PROVIDER` 仅保留用于向后兼容；
    /// 任何非空值都解析为 `Provider::OpenAI`。多 provider 切换由 `AIRP_ENDPOINT`
    /// （指向不同兼容端点）和 `AIRP_MODEL`（指向不同模型 ID）完成。
    pub fn override_with_env(&mut self) {
        if std::env::var("AIRP_PROVIDER").is_ok() {
            self.provider = Provider::OpenAI;
        }
        if let Ok(ep) = std::env::var("AIRP_ENDPOINT") {
            self.endpoint = ep;
        }
        if let Ok(key) = std::env::var("AIRP_API_KEY") {
            self.api_key = Some(key);
        }
        if let Ok(md) = std::env::var("AIRP_MODEL") {
            self.model = md;
        }
        if let Ok(port_str) = std::env::var("AIRP_DAEMON_PORT") {
            if let Ok(port) = port_str.parse::<u16>() {
                self.daemon_port = port;
            }
        }
        if let Ok(k) = std::env::var("AIRP_ACCESS_KEY") {
            if !k.is_empty() {
                self.access_api_key = Some(k);
            }
        }
        if let Ok(e) = std::env::var("AIRP_ENGINE") {
            match e.as_str() {
                "anthropic_messages" => self.engine = BackendEngine::AnthropicMessages,
                "claude_code_sdk" => self.engine = BackendEngine::ClaudeCodeSdk,
                _ => self.engine = BackendEngine::Direct,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_merge_data_settings_overrides_non_empty_fields() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();

        let settings = r#"{
            "endpoint": "https://custom.example.com/v1/chat/completions",
            "model": "custom-model",
            "api_key": "sk-test",
            "daemon_port": 9001
        }"#;
        fs::write(root.join("settings.json"), settings).unwrap();

        let mut cfg = AppConfig::default();
        cfg.merge_data_settings(root).unwrap();

        assert_eq!(
            cfg.endpoint,
            "https://custom.example.com/v1/chat/completions"
        );
        assert_eq!(cfg.model, "custom-model");
        assert!(
            cfg.api_key.is_none(),
            "settings.json secrets must be ignored"
        );
        assert_eq!(cfg.daemon_port, 9001);
    }

    #[test]
    fn test_merge_data_settings_empty_strings_ignored() {
        // 默认模板里 endpoint/api_key 是 ""，不应抹掉程序默认 endpoint
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let settings = r#"{ "endpoint": "", "api_key": "", "model": "" }"#;
        fs::write(root.join("settings.json"), settings).unwrap();

        let default_endpoint = AppConfig::default().endpoint.clone();
        let default_model = AppConfig::default().model.clone();

        let mut cfg = AppConfig::default();
        cfg.merge_data_settings(root).unwrap();

        assert_eq!(cfg.endpoint, default_endpoint);
        assert_eq!(cfg.model, default_model);
        assert!(cfg.api_key.is_none());
    }

    #[test]
    fn test_load_config_ignores_legacy_plaintext_secrets() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("config.json");
        let mut raw = serde_json::to_value(AppConfig::default()).unwrap();
        raw["api_key"] = serde_json::json!("sk-legacy");
        raw["access_api_key"] = serde_json::json!("daemon-secret");
        fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

        let cfg = AppConfig::load_or_create(&path).unwrap();
        assert!(cfg.api_key.is_none());
        assert!(cfg.access_api_key.is_none());
    }

    #[test]
    fn test_serialized_config_omits_secrets() {
        let cfg = AppConfig {
            api_key: Some("sk-runtime".to_string()),
            access_api_key: Some("daemon-runtime".to_string()),
            ..AppConfig::default()
        };
        let raw = serde_json::to_value(cfg).unwrap();
        assert!(raw.get("api_key").is_none());
        assert!(raw.get("access_api_key").is_none());
    }

    #[test]
    fn test_merge_data_settings_ignores_unknown_fields() {
        // settings.json 里允许出现非 AppConfig 的字段（如 default_user_name），不应报错
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let settings = r#"{
            "daemon_port": 8123,
            "default_user_name": "Alice",
            "default_filters": ["<thought>"]
        }"#;
        fs::write(root.join("settings.json"), settings).unwrap();

        let mut cfg = AppConfig::default();
        cfg.merge_data_settings(root).unwrap();
        assert_eq!(cfg.daemon_port, 8123);
    }

    #[test]
    fn test_merge_data_settings_missing_file_is_noop() {
        let tmp = tempdir().unwrap();
        let mut cfg = AppConfig::default();
        let default_port = cfg.daemon_port;
        cfg.merge_data_settings(tmp.path()).unwrap();
        assert_eq!(cfg.daemon_port, default_port);
    }

    #[test]
    fn test_volume_config_validate_ok_on_default() {
        // 默认配置必须通过校验
        assert!(VolumeConfig::default().validate().is_ok());
    }

    #[test]
    fn test_volume_config_validate_rejects_soft_ge_hard() {
        let mut v = VolumeConfig {
            soft_threshold_tokens: 4000,
            hard_threshold_tokens: 3500,
            ..VolumeConfig::default()
        };
        let err = v.validate().unwrap_err();
        assert!(err.contains("soft_threshold_tokens"));
        assert!(err.contains("hard_threshold_tokens"));

        // 相等也拒绝（应严格小于）
        v.soft_threshold_tokens = 3500;
        v.hard_threshold_tokens = 3500;
        assert!(v.validate().is_err());
    }

    #[test]
    fn test_volume_config_validate_rejects_bad_temperature() {
        let mut v = VolumeConfig {
            seal_temperature: -0.1,
            ..VolumeConfig::default()
        };
        assert!(v.validate().is_err());
        v.seal_temperature = 2.1;
        assert!(v.validate().is_err());
    }

    #[test]
    fn test_volume_config_validate_rejects_zero_interval() {
        let v = VolumeConfig {
            maintenance_interval: 0,
            ..VolumeConfig::default()
        };
        assert!(v.validate().is_err());
    }

    #[test]
    fn test_app_config_validate_propagates_volume_failure() {
        let mut cfg = AppConfig::default();
        cfg.volume.soft_threshold_tokens = 10_000;
        cfg.volume.hard_threshold_tokens = 5_000;
        assert!(cfg.validate().is_err());
    }

    // ── M4.6：三层合并顺序验收 ──────────────────────────────────────────────
    //
    // 完整顺序：`default → config.json → data/settings.json → env → request body`。
    // request body 覆盖由 `prepare_pipeline` 在请求层做，已在 chat_pipeline.rs 测试覆盖。
    // 本节验证前 4 层 + 优先级链。
    //
    // 注：`override_with_env` 读全局环境变量，跨测试存在串扰风险，所以下方所有
    // 涉及 env 的测试用一个进程内 Mutex 串行化，保证 set_var / remove_var 可见性。

    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 在 lock 持有期间清空 / 设置全部 AIRP_* env 并执行 closure。
    fn with_env_vars<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let _g = ENV_LOCK.lock().unwrap();
        // 先全清相关键，再按 vars 设置
        const KEYS: &[&str] = &[
            "AIRP_PROVIDER",
            "AIRP_ENDPOINT",
            "AIRP_API_KEY",
            "AIRP_MODEL",
            "AIRP_DAEMON_PORT",
        ];
        for k in KEYS {
            std::env::remove_var(k);
        }
        for (k, v) in vars {
            if let Some(val) = v {
                std::env::set_var(k, val);
            }
        }
        f();
        for k in KEYS {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn test_env_override_beats_data_settings() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let settings = r#"{
            "endpoint": "https://from-settings.example.com/v1/chat/completions",
            "model": "settings-model",
            "api_key": "sk-from-settings",
            "daemon_port": 7777
        }"#;
        fs::write(root.join("settings.json"), settings).unwrap();

        with_env_vars(
            &[
                (
                    "AIRP_ENDPOINT",
                    Some("https://from-env.example.com/v1/chat/completions"),
                ),
                ("AIRP_MODEL", Some("env-model")),
                ("AIRP_API_KEY", Some("sk-from-env")),
                ("AIRP_DAEMON_PORT", Some("9999")),
            ],
            || {
                let mut cfg = AppConfig::default();
                cfg.merge_data_settings(root).unwrap();
                cfg.override_with_env();

                // env 覆盖 settings.json
                assert_eq!(
                    cfg.endpoint,
                    "https://from-env.example.com/v1/chat/completions"
                );
                assert_eq!(cfg.model, "env-model");
                assert_eq!(cfg.api_key.as_deref(), Some("sk-from-env"));
                assert_eq!(cfg.daemon_port, 9999);
            },
        );
    }

    #[test]
    fn test_full_chain_default_then_settings_then_env() {
        // 验证三层叠加：default 提供基底；settings 覆盖部分；env 覆盖更高优先字段
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // settings 只设 endpoint + model
        let settings = r#"{
            "endpoint": "https://from-settings.example.com/v1",
            "model": "settings-model"
        }"#;
        fs::write(root.join("settings.json"), settings).unwrap();

        with_env_vars(
            &[
                // env 只覆盖 model + api_key
                ("AIRP_MODEL", Some("env-only-model")),
                ("AIRP_API_KEY", Some("sk-env-only")),
            ],
            || {
                let default_port = AppConfig::default().daemon_port;
                let mut cfg = AppConfig::default();
                cfg.merge_data_settings(root).unwrap();
                cfg.override_with_env();

                // endpoint 来自 settings（env 未设）
                assert_eq!(cfg.endpoint, "https://from-settings.example.com/v1");
                // model 来自 env（settings 也设了但被 env 覆盖）
                assert_eq!(cfg.model, "env-only-model");
                // api_key 来自 env（settings 未设）
                assert_eq!(cfg.api_key.as_deref(), Some("sk-env-only"));
                // daemon_port 全链未设 → 保持 default
                assert_eq!(cfg.daemon_port, default_port);
            },
        );
    }

    #[test]
    fn test_env_invalid_port_does_not_panic_keeps_previous() {
        // AIRP_DAEMON_PORT 非法（非数字）应静默忽略而非 panic
        with_env_vars(&[("AIRP_DAEMON_PORT", Some("not-a-port"))], || {
            let mut cfg = AppConfig::default();
            let before = cfg.daemon_port;
            cfg.override_with_env();
            assert_eq!(cfg.daemon_port, before, "非法端口字符串应被忽略");
        });
    }

    #[test]
    fn test_volume_config_nested_merge_from_settings() {
        // settings.json 提供 volume 段时，整段替换
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let settings = r#"{
            "volume": {
                "soft_threshold_tokens": 100,
                "hard_threshold_tokens": 200,
                "seal_temperature": 0.5,
                "seal_model": null,
                "maintenance_interval": 5
            }
        }"#;
        fs::write(root.join("settings.json"), settings).unwrap();

        let mut cfg = AppConfig::default();
        cfg.merge_data_settings(root).unwrap();
        assert_eq!(cfg.volume.soft_threshold_tokens, 100);
        assert_eq!(cfg.volume.hard_threshold_tokens, 200);
        assert_eq!(cfg.volume.seal_temperature, 0.5);
        assert_eq!(cfg.volume.maintenance_interval, 5);
        // 合并后 validate 必须通过
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_invalid_volume_in_settings_caught_by_validate() {
        // settings.json 提供 soft >= hard 的违法卷配置 → merge 成功但 validate 失败
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let settings = r#"{
            "volume": {
                "soft_threshold_tokens": 5000,
                "hard_threshold_tokens": 1000,
                "seal_temperature": 0.3,
                "seal_model": null,
                "maintenance_interval": 20
            }
        }"#;
        fs::write(root.join("settings.json"), settings).unwrap();

        let mut cfg = AppConfig::default();
        cfg.merge_data_settings(root).unwrap();
        assert!(cfg.validate().is_err(), "soft>=hard 必须被 validate 拦截");
    }

    #[test]
    fn test_provider_deserialize_from_settings() {
        // M4.1：Provider 已收缩为单变量 OpenAI；任何字符串值应解析为 OpenAI
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let settings = r#"{ "provider": "OpenAI" }"#;
        fs::write(root.join("settings.json"), settings).unwrap();

        let mut cfg = AppConfig::default();
        cfg.merge_data_settings(root).unwrap();
        assert!(matches!(cfg.provider, Provider::OpenAI));
    }
}
