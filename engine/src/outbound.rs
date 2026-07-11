//! Outbound HTTP client policy (#117 A).
//!
//! 所有携带凭据的 provider 请求（chat / agent / models / volume seal）都必须走本模块
//! 构造的 [`reqwest::Client`]，保证 redirect 不泄露 `Authorization` / `x-api-key` /
//! 自定义 secret header 到 cross-origin 或 scheme/port downgrade 的目标。
//!
//! # 设计
//!
//! reqwest 默认 [`Policy::default()`] 跟随最多 10 次 redirect，并在 cross-host redirect
//! 时剥除一个 sensitive header 白名单（`Authorization` / `Cookie` 等）。但该白名单
//! **不覆盖 `x-api-key`、`anthropic-version` 或自定义 secret header**，且同 host
//! scheme/port 变化（如 https → http downgrade）时 sensitive header 也不会被剥除。
//!
//! 对 RP 引擎这是真实风险：provider endpoint 配错或被劫持时，bearer/x-api-key 可
//! 能被 redirect 带到第三方。本模块用 [`Policy::none`] 拒绝所有 redirect ——reqwest
//! 在该 policy 下**不跟随** 3xx，把响应原样返回给调用方。调用方再用
//! [`classify_redirect_response`] 把 3xx 升级成 typed [`AirpError::Upstream`]，
//! 给出可行动脱敏文案而不是裸 reqwest 内部文本。未来若需支持有限 redirect，
//! 再在此处扩一个 `safe_redirect` policy，**绝不**退回 reqwest 默认。
//!
//! 这一改造不引入第二套 HTTP client 配置真相：全进程仍共用一份 [`reqwest::Client`]
//! （连接池由 `Arc` 内部复用），只把构造从裸 `Client::new()` 收到本模块。
//!
//! # 验证
//!
//! `test_outbound_client_does_not_follow_redirect` 用 wiremock 构造 302 → cross-origin
//! 目标，断言：不跟随（拿到 302 本身）、目标 server 0 次收到 Authorization、
//! classify_redirect_response 升级为 `AirpError::Upstream(302)`。

use crate::error::AirpError;

/// 构造一份 credential-safe 的 outbound [`reqwest::Client`]。
///
/// # 安全行为
///
/// - `redirect::Policy::none()`：不跟随任何 redirect，杜绝 cross-origin / downgrade
///   时携带 Authorization / x-api-key / 自定义 secret header；
/// - 不设置全请求 timeout，避免把长时间 SSE body 在固定时限后截断；连接到响应头、
///   planner 和 models probe 继续使用各调用点已有的有界 timeout；
/// - 不开启 HTTP 2 prior knowledge（provider 多为 HTTPS/1.1，避免误降级）；
/// - 不设 referer / user-agent 追踪头，避免泄露部署指纹。
pub fn outbound_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("reqwest outbound client builder config is valid")
}

/// 检测一次 outbound 响应是否为 redirect（3xx），把 reqwest 原样返回的 3xx 升级为
/// typed [`AirpError::Upstream`]，避免凭据被后续逻辑误跟随或裸文案泄露。
///
/// `redirect::Policy::none()` 下 reqwest **不报 error**，而是把 3xx response 原样
/// 返回。调用方在拿到 `send().await?` 后必须先调本 helper，再走 success/4xx/5xx
/// 分流，否则 3xx 会被当成"非 success"走 generic error 路径，丢失 redirect 语义。
///
/// 返回 `Some(AirpError::Upstream)` 时调用方应直接 `Err` 该值；返回 `None` 表示
/// 非 redirect，调用方继续自己的 success/error 分流。
pub(crate) fn classify_redirect_response(response: &reqwest::Response) -> Option<AirpError> {
    let status = response.status();
    if status.is_redirection() {
        Some(AirpError::Upstream {
            status: status.as_u16(),
            body: format!(
                "outbound policy rejected redirect (status {}); provider endpoint must resolve directly without cross-origin or downgrade redirect",
                status.as_u16()
            ),
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{classify_redirect_response, outbound_client};
    use crate::error::AirpError;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// #117 A：cross-origin redirect 携带 Authorization header 必须被拒。
    /// wiremock 302 → cross-origin target，断言：
    /// - outbound client 不跟随（拿到 302 本身，而非 target 的 200）；
    /// - 目标 server 0 次收到 Authorization header（泄露旁路）；
    /// - classify_redirect_response 升级为 AirpError::Upstream(302)。
    #[tokio::test]
    async fn test_outbound_client_does_not_follow_redirect() {
        let client = outbound_client();

        // 目标 server：断言 Authorization header **未到达**（泄露旁路）。
        let target = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/leak"))
            .and(header("authorization", "Bearer secret-leak"))
            .respond_with(ResponseTemplate::new(200).set_body_string("leaked"))
            .expect(0) // 关键断言：0 次到达 = redirect 未跟随、header 未泄露
            .mount(&target)
            .await;

        // provider server：302 → target（cross-origin），outbound policy 必须拒截。
        let provider = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/leak", target.uri())),
            )
            .mount(&provider)
            .await;

        let resp = client
            .post(format!("{}/v1/chat/completions", provider.uri()))
            .header("Authorization", "Bearer secret-leak")
            .header("x-api-key", "secret-leak")
            .json(&serde_json::json!({"model": "x", "messages": []}))
            .send()
            .await
            .expect("reqwest returns 3xx as-is under Policy::none, not error");

        assert_eq!(
            resp.status().as_u16(),
            302,
            "outbound client must not follow redirect"
        );
        let classified = classify_redirect_response(&resp);
        assert!(
            matches!(classified, Some(AirpError::Upstream { status: 302, .. })),
            "classified error must be Upstream(302), got {:?}",
            classified
        );
    }

    /// 非 redirect 响应不被 classify_redirect_response 升级。
    #[tokio::test]
    async fn test_classify_only_redirects() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ok"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let client = outbound_client();
        let resp = client
            .get(format!("{}/ok", server.uri()))
            .send()
            .await
            .expect("ok");
        assert!(classify_redirect_response(&resp).is_none());
    }
}
