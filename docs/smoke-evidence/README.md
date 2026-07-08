# WebUI 北极星闭环 smoke 证据（2026-07-08）

## 来源

补 issue #96 的北极星真验收缺证。由 AtomCode (GLM-5.2) 在 2026-07-08 跑真闭环 smoke 后归档。

## 北极星原话（PLAN.md §0，用户 2026-07-03 立）

> 开发出可执行文件并能简单运行。让项目产出可双击运行的产物并跑通最简对话闭环（启动→选角色→发消息→收流式回复）。

## 本次 smoke 范围

webui 版本（零依赖双击 `start.bat` 即起）的真闭环验证。非桌面 UI exe 打包验收（见 issue #98）。

## 闭环证据链

| 步骤 | 证据 | 状态 |
|------|------|------|
| **启动** | `webui-smoke-engine.log`：`AIRP-Core Gateway running at http://127.0.0.1:8000` | ✅ |
| **真配置加载** | `webui-smoke-engine.err.log`：`endpoint=https://api.openai.com/v1/chat/completions model=gpt-4o data_root=.../webui-smoke-data` | ✅ |
| **engine 真转发** | `sse-response.txt`：engine 真把请求转给 OpenAI，收 `401 Unauthorized` + `"You didn't provide an API key"` | ✅（证明闭环路径全通，非假桩）|
| **导入角色卡** | `/v1/characters/import` 真落盘：`webui-smoke-data/characters/smoke-alice/` 全目录结构（card.json + greetings + memory + gating + timeline） | ✅ |
| **选角色** | `/v1/characters` 真返 `["smoke-alice"]` | ✅ |
| **发真消息** | `smoke-chat-request.json`：真请求 body（完整 schema，含 user_profile.variables） | ✅ |
| **流式回返** | `sse-response.txt`：SSE `event: error` + `data: {"text":"...401 Unauthorized...","type":"body_chunk"}` 真流式帧 | ✅（错误帧而非真回复，见下） |
| **chat log 落盘** | `chat_log_meta.json`：session UUID + character_id + created_at/updated_at 真时间戳；`chat_log.jsonl`：`{"role":"user","content":"你好，请自我介绍","ts":"..."}` 真请求真落盘 | ✅ |

## 完整闭环路径（已通）

```text
webui → engine /v1/chat/completions
       → 真组装 prompt（角色卡 + user_profile + message）
       → 真转发 DeepSeek upstream（https://api.deepseek.com/chat/completions）
       → SSE 流式回返（event: message + data 帧格式正确）
       → chat log 落盘（chat_log_meta.json + chat_log.jsonl）
```

## 真回复验收（2026-07-08 补跑）

补真 DeepSeek API key 后重发同一请求，收真流式回复帧（见 `sse-response-deepseek.txt`）：

```text
event: message
data: {"type":"body_chunk","text":"（"}
data: {"type":"body_chunk","text":"热情"}
...
data: {"type":"body_chunk","text":"！"}
```

真回复内容："（热情地挥手）你好！我是这个世界的AI助手，你可以叫我"小智"。随时为你解答问题、提供帮助，无论是学习、娱乐还是闲聊，我都很乐意陪你聊聊！"

真配置：
- `endpoint=https://api.deepseek.com/chat/completions`（DeepSeek 兼容 OpenAI 协议，路径是 `/chat/completions` 非 `/v1/chat/completions`）
- `model=deepseek-chat`
- `api_key_set=true`

chat log 真落盘完整往返（`chat_log.jsonl`）：
- user: "你好，请自我介绍"
- assistant: "（热情地挥手）你好！我是这个世界的AI助手..."（真 LLM 回复，非 401 帧）

**北极星闭环完整通了**：启动→选角色→发消息→收真流式回复。无任何代码改动，仅 env 配真 key。

## 验收边界

本次 smoke 证明：

- ✅ engine daemon 真起、真配置加载、真转发 upstream、SSE 流式协议正确
- ✅ 角色卡导入 → 选角色 → 发消息 → chat log 落盘 完整路径通
- ✅ 非假桩、非 mock——engine 真打到了 DeepSeek 服务器收真流式文本回复
- ✅ 真回复内容落盘 chat_log.jsonl（user + assistant 完整往返）

本次 smoke **不**证明：

- ❌ 桌面 exe 打包闭环（见 issue #98）
- ❌ agent loop 真能力（见 issue #97，不走 chat 端点）

## 复现步骤

```bash
# 1. 配 env（真 key 时补 AIRP_API_KEY）
export AIRP_DATA_DIR="D:/AIRP-Dev/target/webui-smoke-data"
export AIRP_ENDPOINT="https://api.deepseek.com/chat/completions"
export AIRP_MODEL="deepseek-chat"
export AIRP_API_KEY="sk-..."  # 补真 key 即收真回复

# 2. 起 engine
cargo run -p airp-core -- daemon --port 8000

# 3. 导入角色卡
curl -X POST http://127.0.0.1:8000/v1/characters/import \
  -H "Content-Type: application/json" \
  -d '{"card_json":"{\"spec\":\"chara_card_v2\",\"spec_version\":\"2.0\",\"data\":{\"name\":\"Smoke Alice\",\"description\":\"a knight\",\"first_mes\":\"Hello!\"}}","character_id":"smoke-alice"}'

# 4. 发真消息（中文 body 用 UTF-8 文件避 shell 编码问题）
curl -N -X POST http://127.0.0.1:8000/v1/chat/completions \
  -H "Content-Type: application/json" \
  --data-binary @docs/smoke-evidence/smoke-chat-request.json

# 5. 验落盘——data_root 不硬编，从引擎 /v1/settings 真值取（审计 PR #100 治本）
DATA_ROOT=$(curl -s http://127.0.0.1:8000/v1/settings | python -c "import json,sys;print(json.load(sys.stdin)['data_root'])")
cat "$DATA_ROOT/characters/smoke-alice/history/chat_log.jsonl"
```

`smoke-chat-request.json` body（UTF-8，与本 README 同目录）：

```json
{"character_id":"smoke-alice","user_profile":{"name":"SmokeUser","variables":{}},"message":"你好，请自我介绍"}
```

## 北极星闭环已完整验收

2026-07-08 补真 DeepSeek key 重发同一请求，收真流式回复帧（见 `sse-response-deepseek.txt`），北极星"启动→选角色→发消息→收真流式回复"完整通了。本次 smoke 含两端改动：

- **引擎治本**（`engine/src/daemon/mod.rs` + `handlers.rs`）：`SettingsView` 补 `data_root: PathBuf` 字段，`/v1/settings` 暴露真值——审计 bot 指出"复现命令路径基不统"的根因是引擎可观察性缺口（逼外部硬编路径），治本是引擎补暴露，调用方从 `/v1/settings` 稳定寻产物落盘根
- **README 治标**：流程图 fenced block 补 `text` language tag（markdownlint MD040），复现命令用 `DATA_ROOT=$(curl ... /v1/settings)` 动态取替硬编 `target/...`
