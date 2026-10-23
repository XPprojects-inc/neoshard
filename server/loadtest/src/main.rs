//! Gateway load test.
//!
//!   neoshard-loadtest seed    --db $DATABASE_URL --sessions 100000
//!   neoshard-loadtest connect --url ws://127.0.0.1:8080/gateway --sessions 100000 \
//!                             --source-ips 127.0.0.1,127.0.0.2,... --api http://127.0.0.1:8080
//!
//! `seed` creates users (20 sessions per user, the per-user cap), one guild with
//! all of them and a channel. `connect` opens the sessions (spread over several
//! source IPs to avoid ephemeral-port exhaustion), keeps heartbeats going, then
//! posts messages and measures fan-out latency to every connected session.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use neoshard_proto::framing;
use neoshard_proto::v1::{self, event::Kind, gateway_frame::Kind as F};
use tokio::net::TcpSocket;
use tokio_tungstenite::tungstenite::Message;

const GUILD: i64 = 777;
const CHANNEL: i64 = 778;
const PER_USER: u64 = 20;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Seed {
        #[arg(long, env = "DATABASE_URL")]
        db: String,
        #[arg(long, default_value_t = 1000)]
        sessions: u64,
    },
    Connect {
        #[arg(long, default_value = "ws://127.0.0.1:8080/gateway?v=1&compress=zstd")]
        url: String,
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        api: String,
        #[arg(long, default_value_t = 1000)]
        sessions: u64,
        #[arg(long, value_delimiter = ',', default_value = "127.0.0.1")]
        source_ips: Vec<IpAddr>,
        /// New connections per second.
        #[arg(long, default_value_t = 2000)]
        rate: u64,
        /// Messages to post for the fan-out measurement.
        #[arg(long, default_value_t = 5)]
        messages: u32,
        /// Seconds to hold all connections before measuring.
        #[arg(long, default_value_t = 10)]
        hold: u64,
    },
}

fn token(i: u64) -> String {
    format!("ns_loadtest_{i}")
}

async fn seed(db: &str, sessions: u64) -> anyhow::Result<()> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(db)
        .await?;
    let users = sessions.div_ceil(PER_USER) as i64;
    sqlx::query("INSERT INTO users (id, username) SELECT 1000000 + i, 'lt_' || i FROM generate_series(0, $1 - 1) i ON CONFLICT DO NOTHING")
        .bind(users)
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO guilds (id, name, owner_id) VALUES ($1, 'loadtest', 1000000) ON CONFLICT DO NOTHING").bind(GUILD).execute(&pool).await?;
    sqlx::query("INSERT INTO roles (id, guild_id, name, position, permissions) VALUES ($1, $1, '@everyone', 0, $2) ON CONFLICT DO NOTHING")
        .bind(GUILD)
        .bind(neoshard_proto::permissions::Permissions::DEFAULT_EVERYONE.bits() as i64)
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO channels (id, guild_id, type, name) VALUES ($1, $2, 1, 'flood') ON CONFLICT DO NOTHING").bind(CHANNEL).bind(GUILD).execute(&pool).await?;
    sqlx::query("INSERT INTO members (guild_id, user_id) SELECT $1, 1000000 + i FROM generate_series(0, $2 - 1) i ON CONFLICT DO NOTHING")
        .bind(GUILD)
        .bind(users)
        .execute(&pool)
        .await?;
    // Token hashes computed in SQL with the built-in sha256().
    sqlx::query(
        "INSERT INTO sessions (id, user_id, token_hash, device)
         SELECT 5000000 + i, 1000000 + i / $2, sha256(convert_to('ns_loadtest_' || i, 'UTF8')), 'loadtest'
         FROM generate_series(0, $1 - 1) i ON CONFLICT DO NOTHING",
    )
    .bind(sessions as i64)
    .bind(PER_USER as i64)
    .execute(&pool)
    .await?;
    println!("seeded {users} users, {sessions} sessions, guild {GUILD}, channel {CHANNEL}");
    Ok(())
}

#[derive(Default)]
struct Stats {
    connected: AtomicUsize,
    failed: AtomicUsize,
    dropped: AtomicUsize,
    acks: AtomicU64,
    /// marker message id → receive count
    received: AtomicU64,
    lat_sum_us: AtomicU64,
    lat_max_us: AtomicU64,
}

fn frame(kind: F) -> Message {
    Message::Binary(framing::encode(&v1::GatewayFrame { kind: Some(kind) }, false).into())
}

async fn session(
    i: u64,
    url: String,
    src: IpAddr,
    stats: Arc<Stats>,
    sent_at: Arc<std::sync::Mutex<std::collections::HashMap<String, Instant>>>,
) {
    let fail = |stats: &Stats| {
        stats.failed.fetch_add(1, Ordering::Relaxed);
    };
    let host = url
        .split("//")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or("127.0.0.1:8080")
        .to_string();
    let Ok(addr) = host.parse::<SocketAddr>() else {
        return fail(&stats);
    };
    let Ok(sock) = (if src.is_ipv4() {
        TcpSocket::new_v4()
    } else {
        TcpSocket::new_v6()
    }) else {
        return fail(&stats);
    };
    if sock.bind(SocketAddr::new(src, 0)).is_err() {
        return fail(&stats);
    }
    let Ok(stream) = sock.connect(addr).await else {
        return fail(&stats);
    };
    let Ok((ws, _)) = tokio_tungstenite::client_async(url.as_str(), stream).await else {
        return fail(&stats);
    };
    let (mut tx, mut rx) = ws.split();
    // Hello
    let interval = match rx.next().await {
        Some(Ok(Message::Binary(b))) => match framing::decode(&b).ok().and_then(|f| f.kind) {
            Some(F::Hello(h)) => h.heartbeat_interval_ms,
            _ => return fail(&stats),
        },
        _ => return fail(&stats),
    };
    if tx
        .send(frame(F::Identify(v1::Identify {
            token: token(i),
            intents: 0,
            properties: None,
            initial_status: 1,
        })))
        .await
        .is_err()
    {
        return fail(&stats);
    }
    let mut seq = 0u64;
    let mut ready = false;
    let mut hb = tokio::time::interval(Duration::from_millis(u64::from(interval)));
    hb.tick().await;
    loop {
        tokio::select! {
            _ = hb.tick() => {
                if tx.send(frame(F::Heartbeat(v1::Heartbeat { last_seq: seq }))).await.is_err() { break; }
            }
            msg = rx.next() => {
                let Some(Ok(Message::Binary(b))) = msg else { break };
                let Ok(f) = framing::decode(&b) else { continue };
                match f.kind {
                    Some(F::Ready(_)) if !ready => { ready = true; stats.connected.fetch_add(1, Ordering::Relaxed); }
                    Some(F::HeartbeatAck(_)) => { stats.acks.fetch_add(1, Ordering::Relaxed); }
                    Some(F::Dispatch(d)) => {
                        for item in d.items {
                            seq = item.seq;
                            if let Some(Kind::MessageCreate(m)) = item.event.and_then(|e| e.kind) {
                                let t0 = sent_at.lock().unwrap().get(&m.content).copied();
                                if let Some(t0) = t0 {
                                    let us = t0.elapsed().as_micros() as u64;
                                    stats.received.fetch_add(1, Ordering::Relaxed);
                                    stats.lat_sum_us.fetch_add(us, Ordering::Relaxed);
                                    stats.lat_max_us.fetch_max(us, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    Some(F::Error(_)) | Some(F::InvalidSession(_)) => break,
                    _ => {}
                }
            }
        }
    }
    if ready {
        stats.connected.fetch_sub(1, Ordering::Relaxed);
        stats.dropped.fetch_add(1, Ordering::Relaxed);
    } else {
        fail(&stats);
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Args::parse().cmd {
        Cmd::Seed { db, sessions } => seed(&db, sessions).await,
        Cmd::Connect {
            url,
            api,
            sessions,
            source_ips,
            rate,
            messages,
            hold,
        } => {
            let stats = Arc::new(Stats::default());
            let sent_at = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
            let start = Instant::now();
            let pace = Duration::from_micros(1_000_000 / rate.max(1));
            let mut tick = tokio::time::interval(pace);
            for i in 0..sessions {
                tick.tick().await;
                let src = source_ips[(i as usize) % source_ips.len()];
                tokio::spawn(session(i, url.clone(), src, stats.clone(), sent_at.clone()));
                if i % 10_000 == 0 && i > 0 {
                    println!(
                        "{:>6.1}s spawned {i}, ready {}, failed {}",
                        start.elapsed().as_secs_f64(),
                        stats.connected.load(Ordering::Relaxed),
                        stats.failed.load(Ordering::Relaxed)
                    );
                }
            }
            // Wait for the ramp to settle.
            let mut last = 0;
            for _ in 0..120 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let c = stats.connected.load(Ordering::Relaxed);
                let f = stats.failed.load(Ordering::Relaxed);
                if c + f >= sessions as usize || (c == last && c > 0) {
                    break;
                }
                last = c;
            }
            let ramp = start.elapsed();
            println!(
                "ramp done in {:.1}s: ready {}, failed {}",
                ramp.as_secs_f64(),
                stats.connected.load(Ordering::Relaxed),
                stats.failed.load(Ordering::Relaxed)
            );
            tokio::time::sleep(Duration::from_secs(hold)).await;
            let connected = stats.connected.load(Ordering::Relaxed);
            println!(
                "after {hold}s hold: connected {connected}, dropped {}, heartbeat acks {}",
                stats.dropped.load(Ordering::Relaxed),
                stats.acks.load(Ordering::Relaxed)
            );

            // Fan-out: session 0's owner posts; every connected session should receive it.
            let http = reqwest::Client::new();
            for n in 0..messages {
                let marker = format!("lt-{n}-{}", start.elapsed().as_nanos());
                sent_at
                    .lock()
                    .unwrap()
                    .insert(marker.clone(), Instant::now());
                let r = http
                    .post(format!("{api}/api/v1/channels/{CHANNEL}/messages"))
                    .bearer_auth(token(0))
                    .json(&serde_json::json!({ "content": marker }))
                    .send()
                    .await?;
                if !r.status().is_success() {
                    println!("post failed: {}", r.status());
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            let received = stats.received.load(Ordering::Relaxed);
            let expected = connected as u64 * u64::from(messages);
            let avg = stats.lat_sum_us.load(Ordering::Relaxed) / received.max(1);
            println!(
                "fan-out: delivered {received}/{expected} ({:.2}%), avg latency {:.1} ms, max {:.1} ms",
                received as f64 * 100.0 / expected.max(1) as f64,
                avg as f64 / 1000.0,
                stats.lat_max_us.load(Ordering::Relaxed) as f64 / 1000.0
            );
            Ok(())
        }
    }
}
