//! neoshard-cli — a complete NeoShard client in ~150 lines, built on
//! neoshard-core. Everything (gateway, resume, cache, optimistic sends,
//! permissions) comes from the core; this file is only a "UI".
//!
//!   neoshard-cli --server https://neoshard.ru login alice
//!   neoshard-cli --server https://neoshard.ru chat --channel 123456789
//!
//! Lines you type are sent to the channel; `/history` loads older messages,
//! `/quit` exits.

use std::io::Write as _;
use std::sync::Arc;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use neoshard_core::api::{self, AuthPhase, ConnectionState, command, update};
use neoshard_core::native::{NativeClient, SqliteStore};
use neoshard_core::proto::v1::event::Kind;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Parser)]
#[command(
    version,
    about = "Minimal NeoShard terminal client (built on neoshard-core)"
)]
struct Args {
    /// Instance base URL.
    #[arg(long, env = "NEOSHARD_SERVER", default_value = "http://127.0.0.1:8080")]
    server: String,
    /// Offline cache / session file.
    #[arg(long, default_value = "neoshard-cli.db")]
    db: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Log in (password from NEOSHARD_PASSWORD or stdin); the session is stored in --db.
    Login { login: String },
    /// List guilds and channels.
    Channels,
    /// Chat in a channel.
    Chat {
        #[arg(long)]
        channel: u64,
    },
    /// Log out and delete the stored session.
    Logout,
}

fn send(c: &NativeClient, kind: command::Kind) {
    c.send(api::Command {
        request_id: 0,
        kind: Some(kind),
    });
}

fn read_password() -> anyhow::Result<String> {
    if let Ok(p) = std::env::var("NEOSHARD_PASSWORD") {
        return Ok(p);
    }
    print!("Пароль: ");
    std::io::stdout().flush()?;
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s.trim_end().to_string())
}

/// Wait until the core reports a session (restored from --db or after login).
async fn wait_ready(c: &mut NativeClient) -> anyhow::Result<api::UpdateBatch> {
    let mut collected = api::UpdateBatch::default();
    loop {
        let batch = tokio::time::timeout(std::time::Duration::from_secs(20), c.updates.recv())
            .await
            .context("timed out")?
            .context("core stopped")?;
        for u in &batch.updates {
            match &u.kind {
                Some(update::Kind::Auth(a))
                    if a.phase == AuthPhase::LoggedOut as i32 && !a.error.is_empty() =>
                {
                    bail!("{}", a.error)
                }
                Some(update::Kind::Auth(a)) if a.phase == AuthPhase::MfaRequired as i32 => {
                    print!("Код 2FA: ");
                    std::io::stdout().flush()?;
                    let mut code = String::new();
                    std::io::stdin().read_line(&mut code)?;
                    send(
                        c,
                        command::Kind::Mfa(api::SubmitMfa {
                            code: code.trim().into(),
                        }),
                    );
                }
                Some(update::Kind::Connection(s))
                    if s.state == ConnectionState::AuthFailed as i32 =>
                {
                    bail!("session expired, run `login` again")
                }
                Some(update::Kind::Connection(s)) if s.state == ConnectionState::Ready as i32 => {
                    collected.updates.extend(batch.updates.clone());
                    return Ok(collected);
                }
                _ => {}
            }
        }
        collected.updates.extend(batch.updates);
    }
}

fn author(m: &neoshard_core::proto::v1::Message) -> String {
    m.author
        .as_ref()
        .map(|a| {
            if a.display_name.is_empty() {
                a.username.clone()
            } else {
                a.display_name.clone()
            }
        })
        .unwrap_or_else(|| "?".into())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let store = Arc::new(SqliteStore::open(&args.db)?);
    let mut c = NativeClient::spawn("neoshard-cli", store);

    if let Cmd::Login { login } = &args.cmd {
        let password = read_password()?;
        send(
            &c,
            command::Kind::Login(api::Login {
                api_base: args.server.clone(),
                login: login.clone(),
                password,
            }),
        );
    }
    let initial = wait_ready(&mut c).await?;
    let ready = initial.updates.iter().rev().find_map(|u| {
        if let Some(update::Kind::Ready(r)) = &u.kind {
            Some(r.clone())
        } else {
            None
        }
    });

    match args.cmd {
        Cmd::Login { .. } => println!("Вход выполнен. Сессия сохранена в {}", args.db),
        Cmd::Logout => {
            send(&c, command::Kind::Logout(api::Logout {}));
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            println!("Выход выполнен");
        }
        Cmd::Channels => {
            let r = ready.context("no Ready received")?;
            for g in &r.guilds {
                println!("{} ({})", g.name, g.id);
                for ch in g.channels.iter().filter(|c| c.r#type != 3) {
                    println!("  #{:<24} {}", ch.name, ch.id);
                }
            }
            for dm in &r.private_channels {
                println!("ЛС {}", dm.id);
            }
        }
        Cmd::Chat { channel } => {
            send(
                &c,
                command::Kind::SelectChannel(api::SelectChannel {
                    channel_id: channel,
                }),
            );
            send(
                &c,
                command::Kind::LoadHistory(api::LoadHistory {
                    channel_id: channel,
                    before_id: 0,
                    limit: 20,
                }),
            );
            let mut stdin = BufReader::new(tokio::io::stdin()).lines();
            let mut oldest = 0u64;
            loop {
                tokio::select! {
                    line = stdin.next_line() => {
                        let Some(line) = line? else { break };
                        match line.trim() {
                            "/quit" => break,
                            "/history" => send(&c, command::Kind::LoadHistory(api::LoadHistory { channel_id: channel, before_id: oldest, limit: 20 })),
                            "" => {}
                            text => send(&c, command::Kind::SendMessage(api::SendMessage { channel_id: channel, content: text.into(), reply_to_id: 0, attachment_ids: vec![], sticker_id: 0 })),
                        }
                    }
                    batch = c.updates.recv() => {
                        let Some(batch) = batch else { break };
                        for u in batch.updates {
                            match u.kind {
                                Some(update::Kind::History(h)) if h.channel_id == channel => {
                                    for m in &h.messages {
                                        println!("[{}] {}", author(m), m.content);
                                    }
                                    oldest = h.messages.first().map(|m| m.id).unwrap_or(oldest);
                                }
                                Some(update::Kind::Event(e)) => {
                                    if let Some(Kind::MessageCreate(m)) = e.kind
                                        && m.channel_id == channel {
                                            println!("[{}] {}", author(&m), m.content);
                                        }
                                }
                                Some(update::Kind::PendingFailed(f)) => eprintln!("! не отправлено: {}", f.reason),
                                Some(update::Kind::Connection(s)) if s.state != ConnectionState::Ready as i32 => eprintln!("… соединение: {:?}", ConnectionState::try_from(s.state)),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
