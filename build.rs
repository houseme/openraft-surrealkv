fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .btree_map(".")
        .bytes(".")
        .type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]")
        .field_attribute(".", "#[serde(default)]")
        .compile_protos(&["proto/raft.proto"], &["proto"])?;
    Ok(())
}
