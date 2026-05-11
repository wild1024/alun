//! 通知插件：邮件发送（基于 lettre），支持纯文本和 HTML 格式

use async_trait::async_trait;
use alun_core::{Plugin, Result};
use alun_config::NotificationConfig;
use lettre::{
    Transport, Message,
    message::{Mailbox, MultiPart},
    transport::smtp::{authentication::Credentials, SmtpTransport},
};
use tracing::{info, error};

/// 通知插件：SMTP 邮件发送（基于 lettre）
///
/// 从 `NotificationConfig` 读取 SMTP 连接信息，启动时建立连接。
/// 根据发件人邮箱域名自动选择 `STARTTLS` 或默认 `RELAY` 传输方式。
/// 若未启用或不可达，`send_text()` / `send_html()` 降级为跳过（不报错）。
pub struct NotificationPlugin {
    /// SMTP 配置
    config: NotificationConfig,
    /// SMTP transport（未配置时为 None）
    mailer: Option<SmtpTransport>,
    /// 发件人邮箱
    from: Option<Mailbox>,
}

impl NotificationPlugin {
    /// 从配置创建通知插件
    ///
    /// 若 `smtp_host` 为空或 `enabled = false`，则邮件功能降级为跳过模式。
    /// iCloud / Swisscows 等邮箱使用 `STARTTLS` 加密传输，其他使用默认 `RELAY`。
    pub fn from_config(config: &NotificationConfig) -> Self {
        let (mailer, from) = if config.enabled && !config.smtp_host.is_empty() && !config.smtp_user.is_empty() {
            let from_email = if config.from_email.is_empty() { &config.smtp_user } else { &config.from_email };
            let creds = Credentials::new(config.smtp_user.clone(), config.smtp_pass.clone());

            let builder = if from_email.ends_with("@icloud.com") || from_email.ends_with("@swisscows.email") {
                SmtpTransport::starttls_relay(&config.smtp_host)
            } else {
                SmtpTransport::relay(&config.smtp_host)
            };

            let builder = match builder {
                Ok(b) => b.port(config.smtp_port).credentials(creds),
                Err(e) => {
                    error!("SMTP 传输初始化失败: {}", e);
                    return Self { config: config.clone(), mailer: None, from: None };
                }
            };

            let transport = builder.build();

            let from_name = if config.from_name.is_empty() { "系统通知" } else { &config.from_name };
            let from_mb = format!("{} <{}>", from_name, from_email)
                .parse::<Mailbox>()
                .ok();

            (Some(transport), from_mb)
        } else {
            (None, None)
        };

        Self { config: config.clone(), mailer, from }
    }

    /// 发送纯文本邮件
    pub fn send_text(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        if let (Some(ref mailer), Some(ref from)) = (&self.mailer, &self.from) {
            let to_mb: Mailbox = to.parse().map_err(|e| {
                alun_core::Error::Msg(format!("收件人地址无效: {}", e))
            })?;
            let email = Message::builder()
                .from(from.clone())
                .to(to_mb)
                .subject(subject)
                .body(body.to_string())
                .map_err(|e| alun_core::Error::Msg(format!("邮件构建失败: {}", e)))?;

            mailer.send(&email).map_err(|e| {
                error!("邮件发送失败: {}", e);
                alun_core::Error::Msg(format!("邮件发送失败: {}", e))
            })?;
            info!("邮件已发送 to={} subject={}", to, subject);
            Ok(())
        } else {
            info!("邮件功能未配置，跳过发送: to={} subject={}", to, subject);
            Ok(())
        }
    }

    /// 发送 HTML 邮件（同时附带纯文本版本，适配不同邮件客户端）
    ///
    /// `html_body` 为 HTML 格式的邮件正文，内部自动生成纯文本备用版本。
    pub fn send_html(&self, to: &str, subject: &str, html_body: &str) -> Result<()> {
        if let (Some(ref mailer), Some(ref from)) = (&self.mailer, &self.from) {
            let to_mb: Mailbox = to.parse().map_err(|e| {
                alun_core::Error::Msg(format!("收件人地址无效: {}", e))
            })?;
            let plain_text = Self::html_to_text(html_body);
            let email = Message::builder()
                .from(from.clone())
                .to(to_mb)
                .subject(subject)
                .multipart(MultiPart::alternative_plain_html(
                    plain_text,
                    html_body.to_string(),
                ))
                .map_err(|e| alun_core::Error::Msg(format!("邮件构建失败: {}", e)))?;

            mailer.send(&email).map_err(|e| {
                error!("邮件发送失败: {}", e);
                alun_core::Error::Msg(format!("邮件发送失败: {}", e))
            })?;
            info!("HTML 邮件已发送 to={} subject={}", to, subject);
            Ok(())
        } else {
            info!("邮件功能未配置，跳过发送: to={} subject={}", to, subject);
            Ok(())
        }
    }

    /// 邮件功能是否已配置
    pub fn is_configured(&self) -> bool {
        self.mailer.is_some()
    }

    /// 将 HTML 正文转为纯文本（去除标签并解码常见实体）
    fn html_to_text(html: &str) -> String {
        let mut text = String::with_capacity(html.len());
        let mut in_tag = false;

        for ch in html.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => text.push(ch),
                _ => {}
            }
        }

        let text = text
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&nbsp;", " ");

        let mut result = String::with_capacity(text.len());
        let mut prev_was_whitespace = false;
        for ch in text.chars() {
            if ch == '\n' {
                result.push('\n');
                prev_was_whitespace = true;
            } else if ch.is_whitespace() {
                if !prev_was_whitespace {
                    result.push(' ');
                    prev_was_whitespace = true;
                }
            } else {
                result.push(ch);
                prev_was_whitespace = false;
            }
        }

        result.trim().to_string()
    }
}

#[async_trait]
impl Plugin for NotificationPlugin {
    fn name(&self) -> &str { "notification" }

    async fn start(&self) -> Result<()> {
        if self.is_configured() {
            info!("通知插件就绪: SMTP {}:{}", self.config.smtp_host, self.config.smtp_port);
        } else {
            info!("通知插件: 未配置（跳过）");
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> { Ok(()) }
}
