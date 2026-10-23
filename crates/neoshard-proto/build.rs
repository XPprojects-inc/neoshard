use std::path::PathBuf;

use prost::Message;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../spec/proto");
    let files = [
        "neoshard/v1/models.proto",
        "neoshard/v1/gateway.proto",
        "neoshard/v1/core.proto",
    ];
    for f in &files {
        println!("cargo:rerun-if-changed={}", root.join(f).display());
    }

    // Pure-Rust protobuf compiler: no system `protoc` needed (ADR-0004).
    let fds = protox::compile(files, [&root])?;
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptor_bytes = fds.encode_to_vec();
    std::fs::write(out.join("descriptor.bin"), &descriptor_bytes)?;

    prost_build::Config::new()
        .btree_map(["."])
        .compile_fds(fds)?;

    // Canonical proto3 JSON mapping (u64 as strings) for REST.
    pbjson_build::Builder::new()
        .register_descriptors(&descriptor_bytes)?
        .preserve_proto_field_names()
        .build(&[".neoshard"])?;
    Ok(())
}
