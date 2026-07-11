fn main() {
    prost_build::compile_protos(&["proto/transcript_chunk.proto"], &["proto"])
        .expect("compile transcript chunk protobuf schema");
}
