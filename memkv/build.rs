// Build script to compile libart (C) for comparison
use std::env;
use std::path::PathBuf;

fn main() {
    // Check if libart directory exists
    let libart_path = PathBuf::from("../libart");
    if !libart_path.exists() {
        println!("cargo:warning=libart not found, skipping C bindings");
        return;
    }

    // Compile libart
    cc::Build::new()
        .file("../libart/src/art.c")
        .include("../libart/src")
        .opt_level(3)
        .flag("-march=native")
        .flag("-fno-omit-frame-pointer")
        .compile("art");

    println!("cargo:rerun-if-changed=../libart/src/art.c");
    println!("cargo:rerun-if-changed=../libart/src/art.h");
}
