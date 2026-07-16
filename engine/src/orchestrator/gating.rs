use once_cell::sync::Lazy;
use regex::Regex;
use std::fs;
use std::path::Path;

// M0 F-45 / 6.0m：CP 推进阈值常量化（原硬编码 5 / 10）。
// 后续可考虑迁入 AppConfig.gating: GatingConfig 让用户配置。
const SLOTS_PER_CP_2: u32 = 5;
const SLOTS_PER_CP_3: u32 = 10;

static SLOT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"-\s*累计消耗时槽:\s*(\d+)").expect("SLOT_RE"));
static CP_LINE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"-\s*当前关卡:\s*(\S+)").expect("CP_LINE_RE"));
static PROGRESS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"-\s*进度百分比:\s*(\S+)").expect("PROGRESS_RE"));
static CP_LINE_M_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?m)^\s*-\s*当前关卡:\s*(\S+)").expect("CP_LINE_M_RE"));

fn checkpoint_for_slot(slot: u32) -> &'static str {
    if slot >= SLOTS_PER_CP_3 {
        "CP-3"
    } else if slot >= SLOTS_PER_CP_2 {
        "CP-2"
    } else {
        "CP-1"
    }
}

/// Compute the checkpoint that the next chat turn will use without mutating gating files.
pub fn get_next_checkpoint(data_root: &Path, character_id: &str) -> String {
    if character_id.is_empty() {
        return "CP-1".to_string();
    }
    let character = data_root.join("characters").join(character_id);
    let gating = character.join("gating");
    let checkpoints_exist =
        gating.join("checkpoints.md").is_file() || character.join("checkpoints.md").is_file();
    if !checkpoints_exist {
        return "CP-1".to_string();
    }
    let timeline = if gating.join("timeline.md").is_file() {
        gating.join("timeline.md")
    } else {
        character.join("timeline.md")
    };
    let current_slot = fs::read_to_string(timeline)
        .ok()
        .and_then(|content| {
            SLOT_RE
                .captures(&content)
                .and_then(|captures| captures.get(1))
                .and_then(|value| value.as_str().parse::<u32>().ok())
        })
        .unwrap_or(0);
    checkpoint_for_slot(current_slot.saturating_add(1)).to_string()
}

/// 推进时槽并自动触发剧情流转判定。
///
/// **M1.3 角色隔离**：timeline.md / checkpoints.md 现落在
/// `data_root/characters/{character_id}/gating/`（CF-4 起改用 `gating/` 子目录），
/// 不再共用全局副本。`char_gating_dir` 内部处理旧根目录文件迁移。
pub fn advance_timeline_and_checkpoint(data_root: &Path, character_id: &str) {
    if character_id.is_empty() {
        return;
    }
    let gating_dir = match crate::data_dir::char_gating_dir(data_root, character_id) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(err = %e, "无法获取 gating/ 目录，跳过推进");
            return;
        }
    };
    let timeline_path = gating_dir.join("timeline.md");
    let checkpoints_path = gating_dir.join("checkpoints.md");

    let mut current_slot = 0u32;
    let mut timeline_content = String::new();

    if timeline_path.exists() {
        match fs::read_to_string(&timeline_path) {
            Ok(content) => {
                timeline_content = content;
                if let Some(caps) = SLOT_RE.captures(&timeline_content) {
                    // M0 F-43 / 0.10：捕获组 1 在正则定义中是 (\d+) 必有，
                    // 解析失败仅可能在数字溢出 u32 时；记录后归零。
                    match caps.get(1).map(|m| m.as_str()).map(|s| s.parse::<u32>()) {
                        Some(Ok(num)) => current_slot = num,
                        Some(Err(e)) => tracing::warn!(
                            slot_raw = caps.get(1).map(|m| m.as_str()).unwrap_or(""),
                            err = %e,
                            "timeline.md 中累计时槽数字溢出 u32，重置为 0"
                        ),
                        None => {}
                    }
                }
            }
            Err(e) => tracing::warn!(
                path = ?timeline_path,
                err = %e,
                "读取 timeline.md 失败，按累计时槽=0 处理"
            ),
        }
    }

    current_slot += 1;

    let new_slot_str = format!("- 累计消耗时槽: {}", current_slot);
    if SLOT_RE.is_match(&timeline_content) {
        timeline_content = SLOT_RE
            .replace(&timeline_content, new_slot_str.as_str())
            .to_string();
    } else {
        timeline_content = format!(
            "# 时间线与时槽追踪 timeline\n\n## 统计数据\n{}\n\n{}",
            new_slot_str,
            timeline_content.replace("# 时间线与时槽追踪 timeline", "")
        );
    }

    let log_line = format!("- [时槽 {}] 推进了剧情对话一轮。\n", current_slot);
    if timeline_content.contains("## 历史事件日志") {
        timeline_content =
            timeline_content.replace("## 历史事件日志", &format!("## 历史事件日志\n{}", log_line));
    } else {
        timeline_content.push_str(&format!("\n## 历史事件日志\n{}", log_line));
    }

    if let Err(e) = fs::write(&timeline_path, &timeline_content) {
        // M0 F-44 / 6.0l：写失败不再完全静默
        tracing::warn!(path = ?timeline_path, err = %e, "写入 timeline.md 失败");
    }

    if checkpoints_path.exists() {
        if let Ok(mut cp_content) = fs::read_to_string(&checkpoints_path) {
            let current_cp = checkpoint_for_slot(current_slot);

            let progress_percent = if current_slot >= SLOTS_PER_CP_3 {
                "100%".to_string()
            } else if current_slot >= SLOTS_PER_CP_2 {
                "50%".to_string()
            } else {
                format!("{}%", current_slot * 10)
            };

            cp_content = CP_LINE_RE
                .replace(&cp_content, format!("- 当前关卡: {}", current_cp).as_str())
                .to_string();
            cp_content = PROGRESS_RE
                .replace(
                    &cp_content,
                    format!("- 进度百分比: {}", progress_percent).as_str(),
                )
                .to_string();

            if current_slot >= SLOTS_PER_CP_2 {
                cp_content = cp_content.replace("- [ ] CP-1:", "- [x] CP-1:");
            }
            if current_slot >= SLOTS_PER_CP_3 {
                cp_content = cp_content.replace("- [ ] CP-2:", "- [x] CP-2:");
            }

            if let Err(e) = fs::write(&checkpoints_path, &cp_content) {
                // M0 F-44 / 6.0l
                tracing::warn!(path = ?checkpoints_path, err = %e, "写入 checkpoints.md 失败");
            }
        }
    }
}

/// 获取 checkpoints.md 中的当前关卡名。CF-4：路径改为 `gating/checkpoints.md`。
///
/// 兼容：若 `gating/checkpoints.md` 不存在则降级读旧根目录 `checkpoints.md`（兜底，
/// 适用于未走过 `character_dir()` 迁移路径的边缘场景）。
pub fn get_current_checkpoint(data_root: &Path, character_id: &str) -> String {
    if character_id.is_empty() {
        return "CP-1".to_string();
    }
    let char_dir = data_root.join("characters").join(character_id);
    let cp_path = char_dir.join("gating").join("checkpoints.md");
    let read_path = if cp_path.exists() {
        cp_path
    } else {
        let legacy = char_dir.join("checkpoints.md");
        if !legacy.exists() {
            return "CP-1".to_string();
        }
        legacy
    };
    if let Ok(content) = fs::read_to_string(&read_path) {
        if let Some(caps) = CP_LINE_M_RE.captures(&content) {
            return caps[1].to_string();
        }
    }
    "CP-1".to_string()
}

/// 根据当前 CP 对 known.md 进行物理信息隔离过滤。
pub fn load_filtered_known(data_root: &Path, character_id: &str, current_cp: &str) -> String {
    let char_dir = data_root.join("characters").join(character_id);
    let known_path = char_dir.join("known.md");
    if !known_path.exists() {
        return String::new();
    }
    let raw_known = match fs::read_to_string(&known_path) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    let disclosure_path = char_dir.join("disclosure.json");
    if !disclosure_path.exists() {
        return raw_known;
    }

    let disclosure_raw = match fs::read_to_string(&disclosure_path) {
        Ok(c) => c,
        Err(_) => return raw_known,
    };

    let disclosure_json: serde_json::Value = match serde_json::from_str(&disclosure_raw) {
        Ok(j) => j,
        Err(_) => return raw_known,
    };

    let forbidden_keys: Vec<String> = disclosure_json
        .get(current_cp)
        .and_then(|cp_rules| cp_rules.get("forbidden_keys"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    if forbidden_keys.is_empty() {
        return raw_known;
    }

    raw_known
        .lines()
        .filter(|line| !forbidden_keys.iter().any(|k| line.contains(k.as_str())))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_gating_filtering() {
        let tmp = tempfile::tempdir().unwrap();
        let temp_dir = tmp.path();

        let char_dir = temp_dir.join("characters").join("test_npc");
        fs::create_dir_all(&char_dir).unwrap();

        fs::write(
            char_dir.join("checkpoints.md"),
            "# 剧情进度\n- 当前关卡: CP-2\n- 进度: 20%\n",
        )
        .unwrap();

        let known_content = "普通背景信息A\n这是测试NPC的秘密武器，杀伤力极大\n普通背景信息B\n这行是剧透内容，绝对不能说！\n普通背景信息C";
        fs::write(char_dir.join("known.md"), known_content).unwrap();

        let disclosure_content = r#"{
            "CP-1": { "forbidden_keys": ["剧透内容"] },
            "CP-2": { "forbidden_keys": ["秘密武器", "剧透内容"] }
        }"#;
        fs::write(char_dir.join("disclosure.json"), disclosure_content).unwrap();

        let cp = get_current_checkpoint(temp_dir, "test_npc");
        assert_eq!(cp, "CP-2");

        let filtered = load_filtered_known(temp_dir, "test_npc", &cp);
        assert!(filtered.contains("普通背景信息A"));
        assert!(filtered.contains("普通背景信息B"));
        assert!(filtered.contains("普通背景信息C"));
        assert!(!filtered.contains("秘密武器"));
        assert!(!filtered.contains("剧透内容"));
    }

    #[test]
    fn test_timeline_and_checkpoint_advancement() {
        let tmp = tempfile::tempdir().unwrap();
        let temp_dir = tmp.path();
        let char_id = "alice";
        let gating_dir = temp_dir.join("characters").join(char_id).join("gating");
        fs::create_dir_all(&gating_dir).unwrap();

        fs::write(
            gating_dir.join("timeline.md"),
            "# 时间线与时槽追踪 timeline\n- 累计消耗时槽: 0\n## 历史事件日志\n",
        )
        .unwrap();
        fs::write(
            gating_dir.join("checkpoints.md"),
            "# 剧情关卡 checkpoints (CP)\n- 当前关卡: CP-1\n- 进度百分比: 0%\n- [ ] CP-1: 探索期\n- [ ] CP-2: 对峙期\n",
        )
        .unwrap();

        advance_timeline_and_checkpoint(temp_dir, char_id);
        let cp = get_current_checkpoint(temp_dir, char_id);
        assert_eq!(cp, "CP-1");

        let tl = fs::read_to_string(gating_dir.join("timeline.md")).unwrap();
        assert!(tl.contains("- 累计消耗时槽: 1"));

        for _ in 0..4 {
            advance_timeline_and_checkpoint(temp_dir, char_id);
        }
        let cp = get_current_checkpoint(temp_dir, char_id);
        assert_eq!(cp, "CP-2");

        let cp_content = fs::read_to_string(gating_dir.join("checkpoints.md")).unwrap();
        assert!(cp_content.contains("- 当前关卡: CP-2"));
        assert!(cp_content.contains("- [x] CP-1:"));
        assert!(cp_content.contains("- [ ] CP-2:"));

        for _ in 0..5 {
            advance_timeline_and_checkpoint(temp_dir, char_id);
        }
        let cp = get_current_checkpoint(temp_dir, char_id);
        assert_eq!(cp, "CP-3");

        let cp_content2 = fs::read_to_string(gating_dir.join("checkpoints.md")).unwrap();
        assert!(cp_content2.contains("- 当前关卡: CP-3"));
        assert!(cp_content2.contains("- [x] CP-2:"));
    }

    #[test]
    fn test_cf4_migrate_legacy_gating_files() {
        // CF-4：旧 characters/{id}/checkpoints.md / timeline.md 应迁移到 gating/
        let tmp = tempfile::tempdir().unwrap();
        let temp_dir = tmp.path();
        let char_id = "carol";
        let char_dir = temp_dir.join("characters").join(char_id);
        fs::create_dir_all(&char_dir).unwrap();

        // 旧根目录写文件
        fs::write(
            char_dir.join("checkpoints.md"),
            "# 旧 CP\n- 当前关卡: CP-2\n- 进度百分比: 50%\n",
        )
        .unwrap();
        fs::write(
            char_dir.join("timeline.md"),
            "# 旧 timeline\n- 累计消耗时槽: 5\n",
        )
        .unwrap();

        // 调一次 advance：会先 ensure gating/ 触发迁移，再推进
        advance_timeline_and_checkpoint(temp_dir, char_id);

        // 迁移结果验证
        let gating = char_dir.join("gating");
        assert!(
            gating.join("checkpoints.md").exists(),
            "checkpoints 应在 gating/"
        );
        assert!(gating.join("timeline.md").exists(), "timeline 应在 gating/");
        assert!(
            !char_dir.join("checkpoints.md").exists(),
            "旧根 checkpoints 应被移走"
        );
        assert!(
            !char_dir.join("timeline.md").exists(),
            "旧根 timeline 应被移走"
        );

        // 推进生效（5 → 6）
        let tl = fs::read_to_string(gating.join("timeline.md")).unwrap();
        assert!(tl.contains("- 累计消耗时槽: 6"), "tl = {}", tl);

        // CP 读取从新位置
        let cp = get_current_checkpoint(temp_dir, char_id);
        assert_eq!(cp, "CP-2");
    }

    #[test]
    fn test_cf4_template_initialized_when_absent() {
        // CF-4：完全新建角色时，gating/ 模板自动初始化
        let tmp = tempfile::tempdir().unwrap();
        let temp_dir = tmp.path();
        let char_id = "dave";

        // 不写任何文件，直接 advance（character_dir 路径未走，仅 char_gating_dir 触发）
        advance_timeline_and_checkpoint(temp_dir, char_id);

        let gating = temp_dir.join("characters").join(char_id).join("gating");
        assert!(gating.join("checkpoints.md").exists());
        assert!(gating.join("timeline.md").exists());
        let cp = get_current_checkpoint(temp_dir, char_id);
        assert_eq!(cp, "CP-1"); // 模板默认值
    }

    // AUDIT-4: edge case coverage for gating module

    #[test]
    fn test_audit_4_get_current_checkpoint_empty_id() {
        // Empty character_id must not panic on path join, returns default
        let tmp = tempfile::tempdir().unwrap();
        let cp = get_current_checkpoint(tmp.path(), "");
        assert_eq!(cp, "CP-1");
    }

    #[test]
    fn test_audit_4_get_current_checkpoint_missing_file() {
        // Character dir exists but no checkpoints.md anywhere -> default CP-1
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("characters").join("ghost")).unwrap();
        assert_eq!(get_current_checkpoint(tmp.path(), "ghost"), "CP-1");
    }

    #[test]
    fn test_audit_4_get_current_checkpoint_malformed_returns_default() {
        // checkpoints.md without recognizable CP line -> default CP-1
        let tmp = tempfile::tempdir().unwrap();
        let gating = tmp
            .path()
            .join("characters")
            .join("malformed")
            .join("gating");
        fs::create_dir_all(&gating).unwrap();
        fs::write(
            gating.join("checkpoints.md"),
            "random garbage\nno CP marker",
        )
        .unwrap();
        assert_eq!(get_current_checkpoint(tmp.path(), "malformed"), "CP-1");
    }

    #[test]
    fn test_audit_4_load_filtered_known_no_known_file_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("characters").join("noknown")).unwrap();
        let filtered = load_filtered_known(tmp.path(), "noknown", "CP-1");
        assert_eq!(filtered, "");
    }

    #[test]
    fn test_audit_4_load_filtered_known_no_disclosure_passes_through() {
        // known.md present, disclosure.json missing -> all content returned verbatim
        let tmp = tempfile::tempdir().unwrap();
        let char_dir = tmp.path().join("characters").join("nodis");
        fs::create_dir_all(&char_dir).unwrap();
        fs::write(char_dir.join("known.md"), "secret line 1\nsecret line 2").unwrap();
        let filtered = load_filtered_known(tmp.path(), "nodis", "CP-1");
        assert!(filtered.contains("secret line 1"));
        assert!(filtered.contains("secret line 2"));
    }

    #[test]
    fn test_audit_4_load_filtered_known_malformed_disclosure_passes_through() {
        // Malformed disclosure.json must not crash, returns raw known
        let tmp = tempfile::tempdir().unwrap();
        let char_dir = tmp.path().join("characters").join("bad_disc");
        fs::create_dir_all(&char_dir).unwrap();
        fs::write(char_dir.join("known.md"), "content").unwrap();
        fs::write(char_dir.join("disclosure.json"), "{this is not json").unwrap();
        let filtered = load_filtered_known(tmp.path(), "bad_disc", "CP-1");
        assert_eq!(filtered, "content");
    }

    #[test]
    fn test_audit_4_load_filtered_known_unknown_cp_no_filtering() {
        // disclosure.json valid but lacks entry for current_cp -> no filtering applied
        let tmp = tempfile::tempdir().unwrap();
        let char_dir = tmp.path().join("characters").join("unknown_cp");
        fs::create_dir_all(&char_dir).unwrap();
        fs::write(char_dir.join("known.md"), "line A\nline B").unwrap();
        fs::write(
            char_dir.join("disclosure.json"),
            r#"{"CP-99": {"forbidden_keys": ["A"]}}"#,
        )
        .unwrap();
        // current_cp is CP-1, not in disclosure
        let filtered = load_filtered_known(tmp.path(), "unknown_cp", "CP-1");
        assert!(filtered.contains("line A"));
        assert!(filtered.contains("line B"));
    }

    #[test]
    fn test_audit_4_load_filtered_known_empty_forbidden_keys_passes_all() {
        let tmp = tempfile::tempdir().unwrap();
        let char_dir = tmp.path().join("characters").join("empty_fk");
        fs::create_dir_all(&char_dir).unwrap();
        fs::write(char_dir.join("known.md"), "line A\nline B").unwrap();
        fs::write(
            char_dir.join("disclosure.json"),
            r#"{"CP-1": {"forbidden_keys": []}}"#,
        )
        .unwrap();
        let filtered = load_filtered_known(tmp.path(), "empty_fk", "CP-1");
        assert!(filtered.contains("line A"));
        assert!(filtered.contains("line B"));
    }

    #[test]
    fn test_audit_4_load_filtered_known_substring_match() {
        // Verify forbidden_keys works as substring match, not exact match
        let tmp = tempfile::tempdir().unwrap();
        let char_dir = tmp.path().join("characters").join("substr");
        fs::create_dir_all(&char_dir).unwrap();
        fs::write(
            char_dir.join("known.md"),
            "Alice loves the secret garden.\nBob lives at the manor.",
        )
        .unwrap();
        fs::write(
            char_dir.join("disclosure.json"),
            r#"{"CP-1": {"forbidden_keys": ["secret"]}}"#,
        )
        .unwrap();
        let filtered = load_filtered_known(tmp.path(), "substr", "CP-1");
        assert!(!filtered.contains("Alice"));
        assert!(filtered.contains("Bob"));
    }
}
