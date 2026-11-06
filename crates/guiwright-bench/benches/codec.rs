//! Codec + blob + frame-digest benches (§15.7 micro suite).

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use guiwright_bench::samples;
use guiwright_proto::blob;
use guiwright_proto::schema::{GwpRequest, RequestV1};

fn bench_codec(c: &mut Criterion) {
    let request = GwpRequest::V1(RequestV1::Hello {
        proto_version: 1,
        agent: "bench-client".into(),
    });
    c.bench_function("codec/postcard_hello_roundtrip", |b| {
        b.iter(|| {
            let bytes = postcard::to_allocvec(black_box(&request)).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            black_box(back);
        });
    });
}

fn bench_blob(c: &mut Criterion) {
    let payload = vec![0u8; 8 * 1024]; // above the §15.6 LZ4 threshold
    c.bench_function("blob/lz4_8k_roundtrip", |b| {
        b.iter(|| {
            let compressed = blob::encode_blob(black_box(&payload));
            let back = blob::decode_blob(&compressed).expect("roundtrip");
            black_box(back);
        });
    });
}

fn bench_frame_digest(c: &mut Criterion) {
    c.bench_function("frame_digest/xxh3_10k_vertices", |b| {
        b.iter(|| {
            black_box(samples::frame_digest());
        });
    });
}

criterion_group!(benches, bench_codec, bench_blob, bench_frame_digest);
criterion_main!(benches);
