//! Auto-moderation: keyword, mention-spam and link rules evaluated on every
//! message from members without MANAGE_GUILD.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Json, Router};
use neoshard_proto::permissions::Permissions as P;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;
use crate::authz::{audit, guild_access};
use crate::error::{ApiError, ApiResult, bad};
use crate::extract::Auth;
use crate::routes::messages::Mentions;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/guilds/{gid}/automod/rules", get(list).post(create))
        .route(
            "/guilds/{gid}/automod/rules/{rid}",
            patch(update).delete(remove),
        )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleConfig {
    /// Case-insensitive; `*` matches any run of characters inside a word.
    Keyword {
        keywords: Vec<String>,
        #[serde(default)]
        allow: Vec<String>,
    },
    MentionSpam {
        max: usize,
    },
    /// Block links whose host is not in `allowed_domains` (subdomains included).
    Link {
        #[serde(default)]
        allowed_domains: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Block,
    Timeout,
    Alert,
}

/// Anchored glob over a whole word: `*` matches any run of characters.
fn wildcard_match(pattern: &str, word: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let w: Vec<char> = word.chars().collect();
    let (mut pi, mut wi) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while wi < w.len() {
        if pi < p.len() && p[pi] != '*' && p[pi] == w[wi] {
            pi += 1;
            wi += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = wi;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            wi = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn hosts(content: &str) -> Vec<String> {
    content
        .split(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '(' || c == ')')
        .filter_map(|tok| {
            tok.strip_prefix("https://")
                .or_else(|| tok.strip_prefix("http://"))
        })
        .map(|rest| {
            rest.split(['/', '?', '#', ':'])
                .next()
                .unwrap_or("")
                .to_lowercase()
        })
        .filter(|h| !h.is_empty())
        .collect()
}

/// Pure rule evaluation. Returns true if the message violates the rule.
pub fn violates(rule: &RuleConfig, content: &str, mentions: &Mentions) -> bool {
    match rule {
        RuleConfig::Keyword { keywords, allow } => {
            let lower = content.to_lowercase();
            if allow.iter().any(|a| lower.contains(&a.to_lowercase())) {
                return false;
            }
            keywords.iter().any(|k| {
                let k = k.to_lowercase();
                if k.contains(' ') {
                    lower.contains(&k)
                } else {
                    lower
                        .split(|c: char| !c.is_alphanumeric() && c != '*')
                        .any(|w| !w.is_empty() && wildcard_match(&k, w))
                }
            })
        }
        RuleConfig::MentionSpam { max } => mentions.users.len() + mentions.roles.len() > *max,
        RuleConfig::Link { allowed_domains } => hosts(content).iter().any(|h| {
            !allowed_domains.iter().any(|d| {
                let d = d.to_lowercase();
                *h == d || h.ends_with(&format!(".{d}"))
            })
        }),
    }
}

pub async fn enforce(
    st: &AppState,
    gid: u64,
    author: u64,
    cid: u64,
    content: &str,
    mentions: &Mentions,
) -> ApiResult<()> {
    let rules = sqlx::query!(
        "SELECT id, name, config, action FROM automod_rules WHERE guild_id = $1 AND enabled",
        gid as i64
    )
    .fetch_all(&st.ctx.db)
    .await?;
    for r in rules {
        let Ok(cfg) = serde_json::from_value::<RuleConfig>(r.config) else {
            continue;
        };
        if !violates(&cfg, content, mentions) {
            continue;
        }
        let action: Action = serde_json::from_value(json!(r.action)).unwrap_or(Action::Block);
        audit(st, gid, author, "automod.trigger", author, json!({ "rule": r.name, "rule_id": r.id.to_string(), "channel_id": cid.to_string(), "action": r.action }), "").await?;
        match action {
            Action::Alert => continue,
            Action::Timeout => {
                sqlx::query!(
                    "UPDATE members SET timeout_until = now() + interval '10 minutes' WHERE guild_id = $1 AND user_id = $2",
                    gid as i64,
                    author as i64
                )
                .execute(&st.ctx.db)
                .await?;
                if let Some(m) =
                    neoshard_server_common::repo::member(&st.ctx.db, gid, author).await?
                {
                    st.publish_guild(
                        gid,
                        neoshard_proto::v1::Intent::GuildMembers,
                        neoshard_proto::v1::event::Kind::MemberUpdate(m),
                    );
                }
                return Err(ApiError::Forbidden("message blocked by auto-moderation"));
            }
            Action::Block => return Err(ApiError::Forbidden("message blocked by auto-moderation")),
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct RuleBody {
    name: Option<String>,
    #[serde(flatten)]
    config: Option<RuleConfig>,
    action: Option<Action>,
    enabled: Option<bool>,
}

async fn list(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
) -> ApiResult<Json<serde_json::Value>> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_GUILD)?;
    let rows = sqlx::query!("SELECT id, name, config, action, enabled FROM automod_rules WHERE guild_id = $1 ORDER BY id", gid as i64)
        .fetch_all(&st.ctx.db)
        .await?;
    Ok(Json(json!(rows.into_iter().map(|r| json!({
        "id": r.id.to_string(), "name": r.name, "config": r.config, "action": r.action, "enabled": r.enabled
    })).collect::<Vec<_>>())))
}

async fn create(
    State(st): State<AppState>,
    a: Auth,
    Path(gid): Path<u64>,
    Json(b): Json<RuleBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_GUILD)?;
    let cfg = b.config.ok_or(bad("kind and its config are required"))?;
    let kind = match &cfg {
        RuleConfig::Keyword { keywords, .. } if keywords.is_empty() || keywords.len() > 1000 => {
            return Err(bad("1-1000 keywords"));
        }
        RuleConfig::Keyword { .. } => "keyword",
        RuleConfig::MentionSpam { .. } => "mention_spam",
        RuleConfig::Link { .. } => "link",
    };
    let action = b.action.unwrap_or(Action::Block);
    let id = st.ctx.id();
    let action_str = serde_json::to_value(action)
        .map_err(anyhow::Error::from)?
        .as_str()
        .unwrap_or("block")
        .to_string();
    sqlx::query!(
        "INSERT INTO automod_rules (id, guild_id, name, kind, config, action, enabled) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        id as i64,
        gid as i64,
        b.name.unwrap_or_else(|| kind.to_string()),
        kind,
        serde_json::to_value(&cfg).map_err(anyhow::Error::from)?,
        action_str,
        b.enabled.unwrap_or(true)
    )
    .execute(&st.ctx.db)
    .await?;
    audit(
        &st,
        gid,
        a.id(),
        "automod.rule_create",
        id,
        json!({ "kind": kind }),
        "",
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id.to_string() }))))
}

async fn update(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, rid)): Path<(u64, u64)>,
    Json(b): Json<RuleBody>,
) -> ApiResult<StatusCode> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_GUILD)?;
    let cfg = b
        .config
        .map(serde_json::to_value)
        .transpose()
        .map_err(anyhow::Error::from)?;
    let action = b.action.and_then(|x| {
        serde_json::to_value(x)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
    });
    let r = sqlx::query!(
        "UPDATE automod_rules SET name = COALESCE($3, name), config = COALESCE($4, config), action = COALESCE($5, action), enabled = COALESCE($6, enabled)
         WHERE id = $1 AND guild_id = $2",
        rid as i64,
        gid as i64,
        b.name,
        cfg,
        action,
        b.enabled
    )
    .execute(&st.ctx.db)
    .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    State(st): State<AppState>,
    a: Auth,
    Path((gid, rid)): Path<(u64, u64)>,
) -> ApiResult<StatusCode> {
    guild_access(&st, gid, a.id())
        .await?
        .require(P::MANAGE_GUILD)?;
    sqlx::query!(
        "DELETE FROM automod_rules WHERE id = $1 AND guild_id = $2",
        rid as i64,
        gid as i64
    )
    .execute(&st.ctx.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kw(k: &[&str]) -> RuleConfig {
        RuleConfig::Keyword {
            keywords: k.iter().map(|s| s.to_string()).collect(),
            allow: vec![],
        }
    }

    #[test]
    fn keywords() {
        let none = Mentions::default();
        assert!(violates(&kw(&["спам"]), "Это СПАМ!", &none));
        // Whole-word match: "спамер" is not "спам" without a wildcard.
        assert!(!violates(&kw(&["спам"]), "спамер", &none));
        assert!(violates(&kw(&["спам*"]), "спамер", &none));
        assert!(violates(&kw(&["casino*"]), "best casinoroyale", &none));
        assert!(violates(&kw(&["*coin"]), "buy bitcoin", &none));
        assert!(!violates(&kw(&["cat"]), "dog", &none));
        assert!(violates(&kw(&["free money"]), "get FREE MONEY now", &none));
        let allow = RuleConfig::Keyword {
            keywords: vec!["bad".into()],
            allow: vec!["not bad".into()],
        };
        assert!(!violates(&allow, "this is not bad", &none));
    }

    #[test]
    fn mention_spam() {
        let m = Mentions {
            users: vec![1, 2, 3],
            roles: vec![4],
            everyone: false,
        };
        assert!(violates(&RuleConfig::MentionSpam { max: 3 }, "", &m));
        assert!(!violates(&RuleConfig::MentionSpam { max: 4 }, "", &m));
    }

    #[test]
    fn links() {
        let none = Mentions::default();
        let r = RuleConfig::Link {
            allowed_domains: vec!["neoshard.ru".into()],
        };
        assert!(!violates(&r, "see https://neoshard.ru/docs", &none));
        assert!(!violates(&r, "see https://docs.neoshard.ru", &none));
        assert!(violates(&r, "see http://evil.example/x", &none));
        assert!(violates(&r, "see https://neoshard.ru.evil.example", &none));
        assert!(!violates(&r, "no links here", &none));
    }
}
