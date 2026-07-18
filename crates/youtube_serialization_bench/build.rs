fn main() {
    let protoc =
        protoc_bin_vendored::protoc_bin_path().expect("locate the vendored protobuf compiler");
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc);
    config
        .compile_protos(&["proto/transcript_chunk.proto"], &["proto"])
        .expect("compile transcript chunk protobuf schema");
}
