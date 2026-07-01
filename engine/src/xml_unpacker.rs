//! 流式 XML 标签拆包器：从清洗后文本中分离 `<think>` 心理独白、
//! `<action>` 剧情选项、与剩余正文 Body。
//!
//! 与 [`crate::fsm::StreamingFsm`] 串联使用 — FSM 负责字符级过滤，
//! Unpacker 负责语义分块。两者均为 char-by-char 推进状态机。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
enum UnpackState {
    Normal,
    ReadingOpenTag { buf: String },
    InsideThink,
    InsideThinkReadingCloseTag { buf: String },
    InsideAction,
    InsideActionReadingCloseTag { buf: String },
}

/// 拆包后的语义分块。序列化为前端可消费的 `{type, text}` JSON。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "text")]
pub enum UnpackedChunk {
    /// `<think>...</think>` 标签内文本片段（心理独白，UI 渲染为可折叠框）。
    #[serde(rename = "think_chunk")]
    Think(String),
    /// 标签外的正文文本片段。
    #[serde(rename = "body_chunk")]
    Body(String),
    /// `<action>` 标签解析出的剧情选项数组（UI 渲染为可点击按钮）。
    #[serde(rename = "action_options")]
    ActionOptions {
        /// 各选项文本，按出现顺序。
        options: Vec<String>,
    },
}

/// 字符级流式 XML 拆包状态机。`process_chunk` 接受任意 chunk 边界的输入，
/// `finish()` flush 尾部缓冲。
pub struct StreamingXmlUnpacker {
    state: UnpackState,
    action_buffer: String,
}

impl Default for StreamingXmlUnpacker {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamingXmlUnpacker {
    /// 构造一个新的空 Unpacker（状态 `Normal`，无累积内容）。
    pub fn new() -> Self {
        Self {
            state: UnpackState::Normal,
            action_buffer: String::new(),
        }
    }

    /// Process a stream text token, returning a list of unpacked chunks.
    ///
    /// **M0 F-34 / 6.0i**：批量累积 Body / Think 字符到本地缓冲，仅在状态转换或
    /// chunk 边界时一次性 push，避免每字符一次 `String::new()` + `Vec::push`。
    pub fn process_chunk(&mut self, text: &str) -> Vec<UnpackedChunk> {
        let mut chunks = Vec::new();
        let mut body_buf = String::new();
        let mut think_buf = String::new();

        macro_rules! flush_body {
            ($chunks:expr, $body:expr) => {
                if !$body.is_empty() {
                    $chunks.push(UnpackedChunk::Body(std::mem::take(&mut $body)));
                }
            };
        }
        macro_rules! flush_think {
            ($chunks:expr, $think:expr) => {
                if !$think.is_empty() {
                    $chunks.push(UnpackedChunk::Think(std::mem::take(&mut $think)));
                }
            };
        }

        for c in text.chars() {
            match &mut self.state {
                UnpackState::Normal => {
                    if c == '<' {
                        flush_body!(chunks, body_buf);
                        self.state = UnpackState::ReadingOpenTag {
                            buf: "<".to_string(),
                        };
                    } else {
                        body_buf.push(c);
                    }
                }
                UnpackState::ReadingOpenTag { buf } => {
                    buf.push(c);
                    if buf == "<think>" {
                        self.state = UnpackState::InsideThink;
                    } else if buf == "<action>" {
                        self.state = UnpackState::InsideAction;
                        self.action_buffer.clear();
                    } else if buf == "<content>" || buf == "<talk>" {
                        // Ignore content/talk wrapper tags, go back to Normal
                        self.state = UnpackState::Normal;
                    } else if buf.ends_with('>') {
                        // An unknown closing or opening tag like <something>, output as raw Body
                        body_buf.push_str(buf);
                        self.state = UnpackState::Normal;
                    } else if buf.len() > 15 || c.is_whitespace() {
                        // Recovery: not a valid tag, emit accumulated buffer as Body
                        body_buf.push_str(buf);
                        self.state = UnpackState::Normal;
                    }
                }
                UnpackState::InsideThink => {
                    if c == '<' {
                        flush_think!(chunks, think_buf);
                        self.state = UnpackState::InsideThinkReadingCloseTag {
                            buf: "<".to_string(),
                        };
                    } else {
                        think_buf.push(c);
                    }
                }
                UnpackState::InsideThinkReadingCloseTag { buf } => {
                    buf.push(c);
                    if buf == "</think>" {
                        self.state = UnpackState::Normal;
                    } else if buf.ends_with('>') || buf.len() > 10 {
                        // Fail to match </think>, emit as raw Think content
                        think_buf.push_str(buf);
                        self.state = UnpackState::InsideThink;
                    }
                }
                UnpackState::InsideAction => {
                    if c == '<' {
                        self.state = UnpackState::InsideActionReadingCloseTag {
                            buf: "<".to_string(),
                        };
                    } else {
                        self.action_buffer.push(c);
                    }
                }
                UnpackState::InsideActionReadingCloseTag { buf } => {
                    buf.push(c);
                    if buf == "</action>" {
                        // Try to parse options array from JSON action_buffer
                        if let Ok(val) =
                            serde_json::from_str::<serde_json::Value>(&self.action_buffer)
                        {
                            if let Some(arr) = val.get("options").and_then(|v| v.as_array()) {
                                let options = arr
                                    .iter()
                                    .filter_map(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .collect::<Vec<_>>();
                                // 先 flush 已累积的 body/think，保证顺序
                                flush_body!(chunks, body_buf);
                                flush_think!(chunks, think_buf);
                                chunks.push(UnpackedChunk::ActionOptions { options });
                            }
                        }
                        self.action_buffer.clear();
                        self.state = UnpackState::Normal;
                    } else if buf.ends_with('>') || buf.len() > 10 {
                        // Fail to match </action>, recover back to action_buffer
                        self.action_buffer.push_str(buf);
                        self.state = UnpackState::InsideAction;
                    }
                }
            }
        }

        // chunk 边界 flush 累积的 body / think
        flush_body!(chunks, body_buf);
        flush_think!(chunks, think_buf);

        chunks
    }

    /// Flushes any remaining buffered text at the end of the stream.
    pub fn finish(&mut self) -> Vec<UnpackedChunk> {
        let mut chunks = Vec::new();
        match &self.state {
            UnpackState::ReadingOpenTag { buf } => {
                chunks.push(UnpackedChunk::Body(buf.clone()));
            }
            UnpackState::InsideThinkReadingCloseTag { buf } => {
                for tc in buf.chars() {
                    chunks.push(UnpackedChunk::Think(tc.to_string()));
                }
            }
            UnpackState::InsideActionReadingCloseTag { buf } => {
                self.action_buffer.push_str(buf);
            }
            _ => {}
        }
        self.state = UnpackState::Normal;
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unpacker_basic() {
        let mut unpacker = StreamingXmlUnpacker::new();

        let chunks1 = unpacker.process_chunk("Hello <think>secret thoughts</think> World!");

        // Let's filter out into string categories
        let mut body = String::new();
        let mut think = String::new();

        for c in chunks1 {
            match c {
                UnpackedChunk::Body(t) => body.push_str(&t),
                UnpackedChunk::Think(t) => think.push_str(&t),
                _ => {}
            }
        }

        assert_eq!(body, "Hello  World!");
        assert_eq!(think, "secret thoughts");
    }

    #[test]
    fn test_unpacker_split_tokens() {
        let mut unpacker = StreamingXmlUnpacker::new();

        let mut body = String::new();
        let mut think = String::new();
        let mut actions = Vec::new();

        let tokens = vec![
            "Hello ",
            "<th",
            "ink>",
            "my ",
            "brain",
            "</th",
            "ink>",
            " normal text ",
            "<action>",
            "{\"options\":",
            " [\"Choice A\",",
            " \"Choice B\"]}",
            "</action>",
        ];

        for tok in tokens {
            let chunks = unpacker.process_chunk(tok);
            for c in chunks {
                match c {
                    UnpackedChunk::Body(t) => body.push_str(&t),
                    UnpackedChunk::Think(t) => think.push_str(&t),
                    UnpackedChunk::ActionOptions { options } => actions.extend(options),
                }
            }
        }

        let final_chunks = unpacker.finish();
        for c in final_chunks {
            match c {
                UnpackedChunk::Body(t) => body.push_str(&t),
                UnpackedChunk::Think(t) => think.push_str(&t),
                UnpackedChunk::ActionOptions { options } => actions.extend(options),
            }
        }

        assert_eq!(body, "Hello  normal text ");
        assert_eq!(think, "my brain");
        assert_eq!(
            actions,
            vec!["Choice A".to_string(), "Choice B".to_string()]
        );
    }
}
