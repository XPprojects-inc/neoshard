//! Cost of the core API boundary: encoding/decoding a 64-update batch (what
//! crosses JS↔WASM once per frame) vs. 64 individual crossings (ADR-0009).

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use neoshard_proto::core_api::{Update, UpdateBatch, update};
use neoshard_proto::v1;
use prost::Message;

fn sample_batch(n: usize) -> UpdateBatch {
    let updates = (0..n)
        .map(|i| Update {
            kind: Some(update::Kind::Event(v1::Event {
                kind: Some(v1::event::Kind::MessageCreate(v1::Message {
                    id: 1_000_000 + i as u64,
                    channel_id: 42,
                    guild_id: 7,
                    author: Some(v1::User { id: 9, username: "alice".into(), display_name: "Алиса".into(), ..Default::default() }),
                    content: "Привет! Это обычное сообщение средней длины, с **markdown** и ссылкой https://neoshard.ru".into(),
                    ..Default::default()
                })),
            })),
        })
        .collect();
    UpdateBatch { updates }
}

fn bench(c: &mut Criterion) {
    let batch = sample_batch(64);
    let bytes = batch.encode_to_vec();
    c.bench_function("encode_batch_64", |b| {
        b.iter(|| black_box(&batch).encode_to_vec())
    });
    c.bench_function("decode_batch_64", |b| {
        b.iter(|| UpdateBatch::decode(black_box(bytes.as_slice())).unwrap())
    });
    let singles: Vec<Vec<u8>> = batch.updates.iter().map(|u| u.encode_to_vec()).collect();
    c.bench_function("decode_64_singles", |b| {
        b.iter(|| {
            for s in &singles {
                black_box(Update::decode(s.as_slice()).unwrap());
            }
        })
    });
    c.bench_function("gateway_frame_zstd_roundtrip", |b| {
        let f = v1::GatewayFrame {
            kind: Some(v1::gateway_frame::Kind::Dispatch(v1::DispatchBatch {
                items: batch
                    .updates
                    .iter()
                    .enumerate()
                    .filter_map(|(i, u)| match &u.kind {
                        Some(update::Kind::Event(e)) => Some(v1::Dispatch {
                            seq: i as u64,
                            event: Some(e.clone()),
                        }),
                        _ => None,
                    })
                    .collect(),
            })),
        };
        b.iter(|| {
            let enc = neoshard_proto::framing::encode(black_box(&f), true);
            neoshard_proto::framing::decode(&enc).unwrap()
        })
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
