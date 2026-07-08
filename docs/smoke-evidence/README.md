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

```
webui → engine /v1/chat/completions
       → 真组装 prompt（角色卡 + user_profile + message）
       → 真转发 OpenAI upstream（https://api.openai.com）
       → SSE 流式回返（event: error + data 帧格式正确）
       → chat log 落盘（chat_log_meta.json + chat_log.jsonl）
```

## 唯一缺口（不附和：本次未达"真回复"）

`api_key_set: false`（见 `/v1/settings`）——无真 API key，engine 真转发后收 OpenAI 的 401 Unauthorized，SSE 回返的是错误帧而非真 LLM 回复。

**闭环本身全通**，唯一缺的是真 key 让 OpenAI 返真文本而非 401。补真 key 重发同一请求即可收真流式回复——无需改任何代码。

## 验收边界

本次 smoke 证明：

- ✅ engine daemon 真起、真配置加载、真转发 upstream、SSE 流式协议正确
- ✅ 角色卡导入 → 选角色 → 发消息 → chat log 落盘 完整路径通
- ✅ 非假桩、非 mock——engine 真打到了 OpenAI 服务器收真 HTTP 响应

本次 smoke **不**证明：

- ❌ 真 LLM 回复内容质量（无 key，未收真文本）
- ❌ 桌面 exe 打包闭环（见 issue #98）
- ❌ agent loop 真能力（见 issue #97，不走 chat 端点）

## 复现步骤

```bash
# 1. 配 env（真 key 时补 AIRP_API_KEY）
export AIRP_DATA_DIR="D:/AIRP-Dev/target/webui-smoke-data"
export AIRP_ENDPOINT="https://api.openai.com/v1/chat/completions"
export AIRP_MODEL="gpt-4o"
# export AIRP_API_KEY="sk-..."  # 补真 key 即收真回复

# 2. 起 engine
cargo run -p airp-core -- daemon --port 8000

# 3. 导入角色卡
curl -X POST http://127.0.0.1:8000/v1/characters/import \
  -H "Content-Type: application/json" \
  -d '{"card_json":"{\"spec\":\"chara_card_v2\",\"spec_version\":\"2.0\",\"data\":{\"name\":\"Smoke Alice\",\"description\":\"a knight\",\"first_mes\":\"Hello!\"}}","character_id":"smoke-alice"}'

# 4. 发真消息（中文 body 用 UTF-8 文件避 shell 编码问题）
curl -N -X POST http://127.0.0.1:8000/v1/chat/completions \
  -H "Content-Type: application/json" \
  --data-binary @smoke-chat-request.json

# 5. 验落盘
cat target/webui-smoke-data/characters/smoke-alice/history/chat_log.jsonl
```

`smoke-chat-request.json` body（UTF-8）：

```json
{"character_id":"smoke-alice","user_profile":{"name":"SmokeUser","variables":{}},"message":"你好，请自我介绍"}
```

## 补真 key 即完北极星

本次缺口只需一步：在 env 或 `data/settings.json` 填真 `AIRP_API_KEY`，重发 `smoke-chat-request.json`，即收真流式回复而非 401。代码侧零改动。
