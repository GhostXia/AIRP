//! C-P4 第二批（#484）：widget 宿主 API 版本兼容测试装置（compat harness）。
//!
//! 职责：把「hostApi semver 合同」与「engine policy capability 封闭集」
//! 锁成声明式回归矩阵——任何一处单方面改动（engine 版本语义、文档枚举、
//! 安装面校验）都会在此处亮红灯，强制三方（代码/文档/合同）同步演进。
//!
//! 覆盖矩阵：
//! 1. **解析矩阵**：`parse_host_api_major` 对合法/非法 host_api 值的判定；
//! 2. **安装矩阵**：各 host_api 声明下的安装结局（接受 / 拒绝 + 错误 code）；
//! 3. **前向兼容铁律**：任何非当前 major 的声明一律拒绝，绝不静默尝试；
//! 4. **host_api 往返**：manifest 序列化 → 反序列化 → 语义不变；
//! 5. **文档锁**：`KNOWN_CAPABILITIES` 与 docs/WIDGET-DEVELOPMENT.md §5
//!    的 capability 枚举行严格一致（防「代码改了文档没改」漂移）。
//!
//! 本模块仅在测试构型编译（`#[cfg(test)] mod compat`），不进入产物。

use super::*;

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("airp-extensions-compat")
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

fn request_with_host_api(host_api: Option<&str>) -> InstallRequest {
    InstallRequest {
        manifest: WidgetManifest {
            widget_type: "acme.compat".to_string(),
            version: "1.0.0".to_string(),
            title: None,
            author: None,
            capabilities: vec!["read:state".to_string()],
            host_api: host_api.map(str::to_string),
            entry: WidgetEntry {
                kind: "esm".to_string(),
                source: Some("https://example.com/w.js".to_string()),
                sandbox: true,
            },
        },
        files: vec![file_payload(
            "index.js",
            b"export default () => ({ mount() {} });",
        )],
        slot: None,
    }
}

/// 解析矩阵：(host_api 输入, 期望结局)。
/// 结局二选一——Ok(major) 或 Err(错误 code)。
enum ParseExpectation {
    Major(u32),
    Error(&'static str),
}

fn parse_matrix() -> Vec<(Option<&'static str>, ParseExpectation)> {
    vec![
        // 缺省 / 空串 = 1（向后兼容已有 widget，#489 D1 定夺：文档对齐实现）。
        (None, ParseExpectation::Major(1)),
        (Some(""), ParseExpectation::Major(1)),
        // 合法声明：纯 major / major.minor / major.minor.patch。
        (Some("1"), ParseExpectation::Major(1)),
        (Some("1.0"), ParseExpectation::Major(1)),
        (Some("1.2"), ParseExpectation::Major(1)),
        (Some("1.2.3"), ParseExpectation::Major(1)),
        // 未来 major：解析合法但安装面拒绝（见安装矩阵）。
        (Some("2"), ParseExpectation::Major(2)),
        (Some("999"), ParseExpectation::Major(999)),
        // 非法声明：非数字段 / 前导零 / 段缺数字 / major 0 / 超长。
        (Some("0"), ParseExpectation::Error("invalid_manifest")),
        (Some("01"), ParseExpectation::Error("invalid_manifest")),
        (Some("abc"), ParseExpectation::Error("invalid_manifest")),
        (Some("1.x"), ParseExpectation::Error("invalid_manifest")),
        (Some("1."), ParseExpectation::Error("invalid_manifest")),
        (Some(".1"), ParseExpectation::Error("invalid_manifest")),
        (Some("1..2"), ParseExpectation::Error("invalid_manifest")),
        (
            Some("999999999999"),
            ParseExpectation::Error("invalid_manifest"),
        ),
    ]
}

#[test]
fn compat_parse_matrix() {
    for (input, expectation) in parse_matrix() {
        let result = parse_host_api_major(input);
        match expectation {
            ParseExpectation::Major(major) => {
                assert_eq!(
                    result.unwrap(),
                    major,
                    "host_api = {input:?} 应解析为 major {major}"
                );
            }
            ParseExpectation::Error(code) => {
                assert_eq!(
                    result.unwrap_err().code,
                    code,
                    "host_api = {input:?} 应拒绝（{code}）"
                );
            }
        }
    }
}

/// 安装矩阵：(host_api 声明, 期望结局)。
enum InstallExpectation {
    Accepted,
    Rejected(&'static str),
}

fn install_matrix() -> Vec<(Option<&'static str>, InstallExpectation)> {
    vec![
        // 缺省 / 空串 / 当前 major → 接受（向后兼容）。
        (None, InstallExpectation::Accepted),
        (Some(""), InstallExpectation::Accepted),
        (Some("1"), InstallExpectation::Accepted),
        (Some("1.0"), InstallExpectation::Accepted),
        (Some("1.2.3"), InstallExpectation::Accepted),
        // 跨 major → host_api_incompatible（前向兼容铁律）。
        (
            Some("2"),
            InstallExpectation::Rejected("host_api_incompatible"),
        ),
        (
            Some("2.0.0"),
            InstallExpectation::Rejected("host_api_incompatible"),
        ),
        (
            Some("999"),
            InstallExpectation::Rejected("host_api_incompatible"),
        ),
        // 非法声明 → invalid_manifest。
        (
            Some("abc"),
            InstallExpectation::Rejected("invalid_manifest"),
        ),
        (
            Some("1.x"),
            InstallExpectation::Rejected("invalid_manifest"),
        ),
        (Some("0"), InstallExpectation::Rejected("invalid_manifest")),
    ]
}

#[test]
fn compat_install_matrix() {
    let root = temp_root("install-matrix");
    let store = ExtensionStore::load(root.clone());
    for (declaration, expectation) in install_matrix() {
        // 每轮先清空：同 type 替换语义会让上一轮残留干扰本轮断言。
        for record in store.list() {
            store.remove(&record.id).unwrap();
        }
        let result = store.install(request_with_host_api(declaration));
        match expectation {
            InstallExpectation::Accepted => {
                assert!(
                    result.is_ok(),
                    "host_api = {declaration:?} 应安装成功，实际 {:?}",
                    result.err()
                );
            }
            InstallExpectation::Rejected(code) => {
                assert_eq!(
                    result.unwrap_err().code,
                    code,
                    "host_api = {declaration:?} 应拒绝（{code}）"
                );
                assert!(
                    store.list().is_empty(),
                    "被拒绝的安装不得残留记录（host_api = {declaration:?}）"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 前向兼容铁律：当前 HOST_API_MAJOR 之外的一切 major 都不得被接受。
/// 该测试独立于具体矩阵存在——若未来有人放宽为「低 major 尝试兼容」，
/// 此处强制亮红灯并要求显式修改本断言（合同变更需评审留痕）。
#[test]
fn compat_forward_compat_iron_rule() {
    let root = temp_root("iron-rule");
    let store = ExtensionStore::load(root.clone());
    for other_major in [0u32, HOST_API_MAJOR + 1, HOST_API_MAJOR + 2, 100] {
        // major 0 在安装面先死于 parse（invalid_manifest），其余死于
        // host_api_incompatible——无论哪种，结局必须是拒绝。
        let declaration = other_major.to_string();
        let result = store.install(request_with_host_api(Some(&declaration)));
        assert!(
            result.is_err(),
            "major {other_major} ≠ HOST_API_MAJOR({HOST_API_MAJOR}) 不得被静默接受"
        );
    }
    assert!(store.list().is_empty(), "不兼容安装不得残留");
    let _ = std::fs::remove_dir_all(&root);
}

/// host_api 往返：manifest 序列化 → 反序列化 → 语义不变。
/// 持久化（extensions.json）与 catalog 下发都走 serde 往返，该性质是
/// 「安装时声明 == 下发时声明」的地基。
#[test]
fn compat_host_api_roundtrip() {
    for declaration in [None, Some("1"), Some("1.2.3")] {
        let original = request_with_host_api(declaration).manifest;
        let serialized = serde_json::to_string(&original).unwrap();
        let restored: WidgetManifest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            restored, original,
            "host_api = {declaration:?} 往返必须保真"
        );
        assert_eq!(restored.host_api.as_deref(), declaration);
    }
    // 缺省字段反序列化 → None（与旧记录盘上形态兼容）。
    let legacy = r#"{"type":"acme.legacy","version":"1.0.0","entry":{"kind":"esm","source":"/x.js","sandbox":true}}"#;
    let restored: WidgetManifest = serde_json::from_str(legacy).unwrap();
    assert_eq!(restored.host_api, None, "旧记录缺 host_api 反序列化为 None");
    assert_eq!(
        parse_host_api_major(restored.host_api.as_deref()).unwrap(),
        1
    );
}

/// 文档锁：KNOWN_CAPABILITIES 与 docs/WIDGET-DEVELOPMENT.md §5 的枚举行
/// 严格一致。capability 是对外合同——代码与文档任何一侧单独改动都会
/// 在此失败，强制双侧同步（#484 catalog 完整化的配套守卫）。
#[test]
fn compat_known_capabilities_match_docs() {
    let doc_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("docs")
        .join("WIDGET-DEVELOPMENT.md");
    let content = std::fs::read_to_string(&doc_path)
        .unwrap_or_else(|e| panic!("读取 {} 失败：{e}", doc_path.display()));
    let expected_line = KNOWN_CAPABILITIES.join(" | ");
    assert!(
        content.lines().any(|line| line.trim() == expected_line),
        "docs/WIDGET-DEVELOPMENT.md §5 必须存在与 KNOWN_CAPABILITIES 完全一致的枚举行：\n{expected_line}"
    );
}
