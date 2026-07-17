use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct RegexFilter {
    pub start: String,
    pub end: String,
}

impl RegexFilter {
    /// 尝试从一个正则表达式字符串中解析出起止字符串。
    /// 比如将 `"<thought>[\s\S]*?<\/thought>"` 解析为 start: "<thought>", end: "</thought>"
    /// 将 `"\\[系统提示:[\s\S]*?\\]"` 解析为 start: "[系统提示:", end: "]"
    pub fn from_regex(regex_str: &str) -> Self {
        // 一个简单的解析器：寻找形式如 "START[\s\S]*?END" 的模式
        let parts: Vec<&str> = regex_str.split("[\\s\\S]*?").collect();
        if parts.len() >= 2 {
            let start = unescape_regex(parts[0]);
            let end = unescape_regex(parts[1]);
            RegexFilter { start, end }
        } else {
            let parts2: Vec<&str> = regex_str.split(".*?").collect();
            if parts2.len() >= 2 {
                let start = unescape_regex(parts2[0]);
                let end = unescape_regex(parts2[1]);
                RegexFilter { start, end }
            } else {
                // 如果解析失败，把整个正则作为 start，end 为空
                RegexFilter {
                    start: unescape_regex(regex_str),
                    end: String::new(),
                }
            }
        }
    }
}

// 辅助函数，将正则转义字符恢复为普通字符，例如 `\<` -> `<`, `\[` -> `[`
fn unescape_regex(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                match next {
                    '[' | ']' | '(' | ')' | '{' | '}' | '*' | '+' | '?' | '|' | '^' | '$'
                    | '\\' | '.' | '/' => {
                        result.push(next);
                        chars.next();
                    }
                    _ => {
                        result.push(c);
                    }
                }
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[derive(Debug, Clone, PartialEq)]
enum FsmState {
    Normal,
    // 缓冲匹配前缀状态：包含当前缓冲的内容
    Buffering {
        buffer: String,
    },
    // 变量名收集状态：已经匹配了 `{{`，目前在缓冲变量名
    VariableBuffering {
        var_name_buffer: String,
    },
    // 过滤拦截状态：已经完全匹配了某个 filter.start
    Filtering {
        filter_index: usize,
        end_match_buffer: String,
    },
}

/// 字符级流过滤状态机。`process_chunk` 输入未清洗 chunk，输出清洗后字符串。
///
/// 内部按 char 推进，状态转换严格遵循 `FsmState`。M6.3 proptest 保证 chunk 切分
/// 不影响输出且任意 UTF-8 不 panic。
pub struct StreamingFsm {
    state: FsmState,
    filters: Vec<RegexFilter>,
    variables: HashMap<String, String>,
    special_starts: Vec<String>,
    /// 6.0h：所有 special_starts 的首字符集合，用于 Normal 态批量快进。
    /// 单个 char 命中时再走完整 `is_any_special_start_prefix` 判定。
    special_first_chars: std::collections::HashSet<char>,
}

impl StreamingFsm {
    /// 构造一个新 FSM，传入用户自定义过滤器集合与变量替换表。
    pub fn new(filters: Vec<RegexFilter>, variables: HashMap<String, String>) -> Self {
        let mut special_starts = vec!["{{".to_string()];
        for filter in &filters {
            if !filter.start.is_empty() {
                special_starts.push(filter.start.clone());
            }
        }
        // 6.0h：预计算首字符集合，hot path Normal 态快速跳过普通文本。
        let special_first_chars: std::collections::HashSet<char> = special_starts
            .iter()
            .filter_map(|s| s.chars().next())
            .collect();
        StreamingFsm {
            state: FsmState::Normal,
            filters,
            variables,
            special_starts,
            special_first_chars,
        }
    }

    /// 测试专用：暴露内部过滤器集合，供 issue #27 single/scene 一致性回归断言使用。
    #[cfg(test)]
    pub(crate) fn filters_for_test(&self) -> &[RegexFilter] {
        &self.filters
    }

    /// 输入一个字符，返回可立即输出的字符串。
    pub fn process_char(&mut self, c: char) -> String {
        match &mut self.state {
            FsmState::Normal => {
                let s = c.to_string();
                if self.is_any_special_start_prefix(&s) {
                    if let Some((filter_idx, is_var)) = self.check_full_match(&s) {
                        if is_var {
                            self.state = FsmState::VariableBuffering {
                                var_name_buffer: String::new(),
                            };
                        } else {
                            self.state = FsmState::Filtering {
                                filter_index: filter_idx,
                                end_match_buffer: String::new(),
                            };
                        }
                    } else {
                        self.state = FsmState::Buffering { buffer: s };
                    }
                    String::new()
                } else {
                    s
                }
            }
            FsmState::Buffering { buffer } => {
                // M0 F-31 / 6.0h：`std::mem::take` 取出 owned String，避免 hot path clone。
                let mut temp = std::mem::take(buffer);
                temp.push(c);

                if self.is_any_special_start_prefix(&temp) {
                    if let Some((filter_idx, is_var)) = self.check_full_match(&temp) {
                        if is_var {
                            self.state = FsmState::VariableBuffering {
                                var_name_buffer: String::new(),
                            };
                        } else {
                            self.state = FsmState::Filtering {
                                filter_index: filter_idx,
                                end_match_buffer: String::new(),
                            };
                        }
                    } else {
                        self.state = FsmState::Buffering { buffer: temp };
                    }
                    String::new()
                } else if let Some(suffix_idx) = self.find_longest_special_start_prefix_suffix(&temp) {
                    let to_output = temp[..suffix_idx].to_string();
                    let new_buffer = temp[suffix_idx..].to_string();

                    if let Some((filter_idx, is_var)) = self.check_full_match(&new_buffer) {
                        if is_var {
                            self.state = FsmState::VariableBuffering {
                                var_name_buffer: String::new(),
                            };
                        } else {
                            self.state = FsmState::Filtering {
                                filter_index: filter_idx,
                                end_match_buffer: String::new(),
                            };
                        }
                    } else {
                        self.state = FsmState::Buffering { buffer: new_buffer };
                    }
                    to_output
                } else {
                    self.state = FsmState::Normal;
                    temp
                }
            }
            FsmState::VariableBuffering { var_name_buffer } => {
                // M0 F-31 / 6.0h：同上，避免 clone。
                let mut temp = std::mem::take(var_name_buffer);
                temp.push(c);

                if temp.ends_with("}}") {
                    let var_name = &temp[..temp.len() - 2];
                    let var_name_trimmed = var_name.trim();
                    let replacement = if let Some(val) = self.variables.get(var_name_trimmed) {
                        val.clone()
                    } else {
                        format!("{{{{{}}}}}", var_name)
                    };
                    self.state = FsmState::Normal;
                    replacement
                } else {
                    let is_valid = temp
                        .chars()
                        .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == ' ' || ch == '}');
                    if !is_valid || temp.len() > 64 {
                        let fallback_text = format!("{{{{{}", temp);
                        self.state = FsmState::Normal;
                        fallback_text
                    } else {
                        self.state = FsmState::VariableBuffering {
                            var_name_buffer: temp,
                        };
                        String::new()
                    }
                }
            }
            FsmState::Filtering {
                filter_index,
                end_match_buffer,
            } => {
                let filter = &self.filters[*filter_index];
                // M0 F-31 / 6.0h：同上，避免 clone。
                let mut temp = std::mem::take(end_match_buffer);
                temp.push(c);

                if temp.ends_with(&filter.end) {
                    self.state = FsmState::Normal;
                    String::new()
                } else {
                    let max_chars = filter.end.chars().count();
                    let temp_chars: Vec<char> = temp.chars().collect();
                    if temp_chars.len() > max_chars {
                        let skip = temp_chars.len() - max_chars;
                        temp = temp_chars[skip..].iter().collect();
                    }
                    self.state = FsmState::Filtering {
                        filter_index: *filter_index,
                        end_match_buffer: temp,
                    };
                    String::new()
                }
            }
        }
    }

    /// 输入流数据块，返回清洗替换后的数据块。
    ///
    /// 6.0h 优化：在 `FsmState::Normal` 时，扫描连续非特殊首字符的子串并一次性
    /// 拷贝到输出，避免每字符 `c.to_string() + push_str` 的 N 次 String 分配。
    /// 其它状态（Buffering / VariableBuffering / Filtering）仍逐 char 走，
    /// 因这些状态的 char 必须参与状态转换判定。
    pub fn process_chunk(&mut self, chunk: &str) -> String {
        let mut out = String::with_capacity(chunk.len());
        let mut iter = chunk.char_indices().peekable();
        while let Some(&(idx, c)) = iter.peek() {
            if matches!(self.state, FsmState::Normal) && !self.special_first_chars.contains(&c) {
                // 快进：从当前 idx 起一路收集非特殊首字符的 char，最后一次性 push
                let start = idx;
                let mut end = idx;
                while let Some(&(i, ch)) = iter.peek() {
                    if self.special_first_chars.contains(&ch) {
                        break;
                    }
                    end = i + ch.len_utf8();
                    iter.next();
                }
                out.push_str(&chunk[start..end]);
            } else {
                // 进入或处于状态机内部 — 逐 char 推进
                iter.next();
                let produced = self.process_char(c);
                if !produced.is_empty() {
                    out.push_str(&produced);
                }
            }
        }
        out
    }

    /// 输入流结束时清空缓冲并返回剩余输出。
    pub fn finish(&mut self) -> String {
        // M0 F-31 / 6.0h：用 replace(state, Normal) 避免 clone。
        match std::mem::replace(&mut self.state, FsmState::Normal) {
            FsmState::Normal => String::new(),
            FsmState::Buffering { buffer } => buffer,
            FsmState::VariableBuffering { var_name_buffer } => {
                format!("{{{{{}", var_name_buffer)
            }
            FsmState::Filtering { .. } => String::new(),
        }
    }

    fn is_any_special_start_prefix(&self, s: &str) -> bool {
        for start in &self.special_starts {
            if start.starts_with(s) {
                return true;
            }
        }
        false
    }

    fn check_full_match(&self, s: &str) -> Option<(usize, bool)> {
        if s == "{{" {
            return Some((0, true));
        }
        for (idx, filter) in self.filters.iter().enumerate() {
            if filter.start == s {
                return Some((idx, false));
            }
        }
        None
    }

    fn find_longest_special_start_prefix_suffix(&self, s: &str) -> Option<usize> {
        // 必须按 char 边界切片，否则中文等多字节字符切到一半 → panic。
        for (byte_idx, _) in s.char_indices().skip(1) {
            let suffix = &s[byte_idx..];
            if self.is_any_special_start_prefix(suffix) {
                return Some(byte_idx);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
