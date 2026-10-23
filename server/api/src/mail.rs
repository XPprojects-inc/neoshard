//! Outgoing email via the instance's own SMTP (no foreign SaaS, ADR-0011).

use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::AppState;

/// Best effort: failures are logged, never shown to the requester (no enumeration).
pub async fn send(st: &AppState, to: &str, subject: &str, body: &str) {
    let Some(url) = &st.ctx.cfg.smtp_url else {
        if st.ctx.cfg.dev {
            tracing::warn!(%to, %subject, %body, "dev mode: email not sent (no NEOSHARD_SMTP_URL)");
        } else {
            tracing::error!("cannot send email: NEOSHARD_SMTP_URL is not configured");
        }
        return;
    };
    let result = async {
        let from: Mailbox = st.ctx.cfg.smtp_from.parse()?;
        let msg = Message::builder()
            .from(from)
            .to(to.parse()?)
            .subject(subject)
            .body(body.to_string())?;
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)?.build();
        transport.send(msg).await?;
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e, "email delivery failed");
    }
}
