//! Packs data/blocks.json (the block table generated from the game) into the compact,
//! deflated form that the library embeds; see src/blocktable.rs.

#[path = "src/blocktable.rs"]
mod blocktable;

fn main() {
    let src = "data/blocks.json";
    println!("cargo:rerun-if-changed={src}");
    println!("cargo:rerun-if-changed=src/blocktable.rs");
    let text = std::fs::read_to_string(src).expect("read data/blocks.json");
    let table = blocktable::from_json(&text);
    let bin = blocktable::encode(&table);
    // the packed form must give back exactly the same table (floats compared bit for bit)
    let back = blocktable::decode(&bin);
    assert!(back == table, "block table does not survive packing");
    assert!(blocktable::encode(&back) == bin, "block table packing is not stable");
    let packed = miniz_oxide::deflate::compress_to_vec(&bin, 10);
    assert!(miniz_oxide::inflate::decompress_to_vec(&packed).expect("inflate") == bin);
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("blocks.bin.z");
    std::fs::write(out, packed).expect("write packed block table");
}
