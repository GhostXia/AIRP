use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;

use airp_core::chat_pipeline;
use airp_core::config::{AppConfig, DeploymentMode};
use airp_core::daemon::{
    create_router, ChatCompletionRequest, DaemonState, MutableConfig, UserProfile,
};
use airp_core::error::AirpError;
use airp_core::types::CharacterId;

#[derive(Parser)]
#[command(
    name = "airp-core",
    author = "AIRP Team",
    version = "0.1.0",
    about = "AIRP streaming RP backend (pure Agent side)"
)]
struct Cli {
    /// 配置文件路径，默认会在当前目录下创建/加载 config.json
    #[arg(short, long, default_value = "config.json")]
    config: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
enum Commands {
    /// 启动守护进程 (Daemon SSE 网关)
    Daemon {
        /// 监听端口，若不指定则使用配置文件中的 daemon_port 端口 (默认 8000)
        #[arg(short, long)]
        port: Option<u16>,

        /// 监听地址。默认仅 loopback；容器部署必须显式传入 0.0.0.0，且不得发布 engine 端口。
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
    /// 在终端控制台直接运行单次流式过滤
    Run {
        /// 角色卡：PNG 路径 / JSON 文件路径 / `{...}` 内联 JSON / data/characters/{id} 文件夹名
        #[arg(short, long)]
        character: Option<String>,

        /// 世界书 JSON 文件的任意路径
        #[arg(short, long)]
        lorebook: Option<String>,

        /// 用户当前输入的 Message 提示词
        #[arg(short, long)]
        message: String,

        /// 正则过滤表达式列表 (如: "<thought>[\\s\\S]*?<\\/thought>")
        #[arg(short, long)]
        filters: Vec<String>,

        /// 运行时用户名称，默认为 "User"
        #[arg(long, default_value = "User")]
        user_name: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // M1.9 / M2 前置：初始化 tracing subscriber，让 daemon 里 warn/error/debug 可见。
    // 支持 AIRP_LOG 环境变量自定义级别，默认 info。
    let log_level = std::env::var("AIRP_LOG").unwrap_or_else(|_| "info".to_string());
    // M_MCP MCP-1: tracing 输出必须走 stderr，否则会污染 MCP stdio JSON-RPC 通道。
    // Daemon 的 println! / Run 的流式正文仍走 stdout，互不影响。
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(log_level))
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();

    // 1. 解析命令行参数
    let cli = Cli::parse();

    // 2. 配置三层合并（M4.3）：
    //    layer 1 = AppConfig::default() （程序默认）
    //    layer 2a = `config.json`（向后兼容）
    //    layer 2b = `data/settings.json`（用户层，覆盖上一层非空字段）
    //    layer 3 = 环境变量
    //    layer 4 = HTTP 请求 body（在 chat_pipeline::prepare_pipeline 里处理）
    let mut app_config = AppConfig::load_or_create(&cli.config)?;
    let data_root = airp_core::data_dir::resolve_data_root();
    app_config.merge_data_settings(&data_root)?;
    app_config.override_with_env()?;
    // M0 F-12 / 5.0b：全部合并完成后 fast-fail 校验跨字段不变量
    app_config.validate()?;
    if matches!(&cli.command, Commands::Daemon { .. }) {
        app_config.validate_daemon_startup(&data_root)?;
    }
    airp_core::data_dir::ensure_data_dirs(&data_root)?;
    tracing::info!(
        endpoint = %app_config.endpoint,
        model = %app_config.model,
        has_api_key = app_config.api_key.is_some(),
        data_root = %data_root.display(),
        "config loaded"
    );

    // M0 F-01：构造一份共享 HTTP 客户端，整个进程生命周期复用其连接池。
    // #117 A：必须走 outbound::outbound_client() —— reqwest 默认 redirect policy
    // 会在 cross-origin / scheme downgrade 时携带 Authorization / x-api-key / 自定义
    // secret header，对 provider key 是真实泄露风险。绝不在本仓任何处再用裸
    // `reqwest::Client::new()` 发凭据请求。
    let http_client = airp_core::outbound::outbound_client();

    match cli.command {
        Commands::Daemon { port, host } => {
            let daemon_port = port.unwrap_or(app_config.daemon_port);
            let bind_ip = host
                .parse::<IpAddr>()
                .map_err(|_| "--host must be an IP literal such as 127.0.0.1 or 0.0.0.0")?;
            if !bind_ip.is_loopback() && app_config.deployment_mode != DeploymentMode::Production {
                return Err("non-loopback --host is allowed only in production mode".into());
            }

            let state = Arc::new(DaemonState {
                data_root,
                http_client: http_client.clone(),
                config: std::sync::RwLock::new(MutableConfig {
                    provider: app_config.provider,
                    endpoint: app_config.endpoint,
                    api_key: app_config.api_key,
                    model: app_config.model,
                    volume_config: app_config.volume,
                    access_api_key: app_config.access_api_key,
                    engine: app_config.engine,
                    quota: app_config.quota,
                    deployment_mode: app_config.deployment_mode,
                    public_origin: app_config.public_origin,
                }),
            });

            let router = create_router(state);
            let addr = SocketAddr::new(bind_ip, daemon_port);
            let listener = TcpListener::bind(&addr).await?;

            println!("AIRP-Core Gateway running at http://{}", addr);
            println!("Open your browser and visit the address above.");
            // M6.4：限流层需要 `ConnectInfo<SocketAddr>` 来识别客户端 IP；
            // 用 `into_make_service_with_connect_info` 让 axum 把 peer 地址注入扩展。
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await?;
        }
        Commands::Run {
            character,
            lorebook,
            message,
            filters,
            user_name,
        } => {
            // M4.5：将 character / lorebook 解析为可被 pipeline 直接消费的形式，
            // 然后构造与 daemon 完全相同的 ChatCompletionRequest + DaemonState，
            // 走 prepare_pipeline → run_pipeline_to_stdout 单一路径。

            // A. 角色卡：解析为内联 JSON 字符串（pipeline 识别 `{...}` 开头）
            //    或保留为 data/characters/{id} 文件夹名（CharacterId）。
            //    M5.0a：character_id 用 newtype，构造时即触发 validate_id_segment。
            let (character_id, character_card_id): (Option<CharacterId>, Option<String>) =
                match character {
                    None => (None, None),
                    Some(ref s) if s.trim().starts_with('{') => (None, Some(s.clone())),
                    Some(ref s) if s.to_lowercase().ends_with(".png") => {
                        let json = airp_core::png_parser::parse_png_character_card(s)?;
                        (None, Some(json))
                    }
                    Some(ref s)
                        if std::path::Path::new(s)
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("json"))
                            .unwrap_or(false)
                            && std::path::Path::new(s).exists() =>
                    {
                        let json = std::fs::read_to_string(s)?;
                        (None, Some(json))
                    }
                    Some(s) => (Some(CharacterId::new(s)?), None),
                };

            // B. 世界书：从任意路径读为内联 JSON 传入 `lorebook_path`
            //    （prepare_pipeline 识别 `{...}` 开头跳过路径校验）
            let lorebook_inline: Option<String> = match lorebook {
                None => None,
                Some(ref p) => Some(std::fs::read_to_string(p)?),
            };

            // C. DaemonState：与 daemon 模式相同的字段集合
            let state = Arc::new(DaemonState {
                data_root,
                http_client: http_client.clone(),
                config: std::sync::RwLock::new(MutableConfig {
                    provider: app_config.provider,
                    endpoint: app_config.endpoint,
                    api_key: app_config.api_key,
                    model: app_config.model,
                    volume_config: app_config.volume,
                    access_api_key: None, // Run 命令不涉及 HTTP 鉴权
                    engine: app_config.engine,
                    quota: app_config.quota,
                    deployment_mode: app_config.deployment_mode,
                    public_origin: app_config.public_origin,
                }),
            });

            // D. ChatCompletionRequest
            let payload = ChatCompletionRequest {
                character_id,
                character_card_id,
                lorebook_path: lorebook_inline,
                user_profile: UserProfile {
                    name: user_name,
                    variables: HashMap::new(),
                },
                message,
                messages_history: None,
                regex_filters: if filters.is_empty() {
                    None
                } else {
                    Some(filters)
                },
                preset_id: None,
                enabled_presets: None,
                session_id: None,
                provider: None,
                endpoint: None,
                api_key: None,
                model: None,
                temperature: None,
                max_tokens: None,
                scene_id: None,
                user_id: None,
            };

            // E. 走 pipeline（与 daemon 完全相同路径）
            let pipeline = chat_pipeline::prepare_pipeline(&payload, &state)
                .map_err(|e: AirpError| -> Box<dyn std::error::Error> { Box::new(e) })?;

            chat_pipeline::run_pipeline_to_stdout(pipeline).await?;
        }
    }

    Ok(())
}
