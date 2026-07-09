use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-search={}", manifest_dir);
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");

    // 编译期内嵌 DTB：从 .dts（唯一真源，已追踪）用 dtc 生成 .dtb 到 OUT_DIR，
    // 经 cargo:rustc-env 传路径给 include_bytes!(env!("QEMU_VIRT_DTB_PATH"))。
    // 不再追踪 .dtb 产物——fresh clone 后自动派生，仅需装 dtc。
    // build.rs 在 crate 根，比 src/lib.rs 少一层 ../，故 4 级到 rt-async 根。
    let dts = PathBuf::from(&manifest_dir)
        .join("../../../..")
        .join("its/rt-async-qemu-virt.dts");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dtb = out_dir.join("rt-async-qemu-virt.dtb");

    let out = Command::new("dtc")
        .args([
            "-I",
            "dts",
            "-O",
            "dtb",
            "-o",
            &dtb.to_string_lossy(),
            &dts.to_string_lossy(),
        ])
        .output()
        .unwrap_or_else(|_| {
            panic!(
                "dtc not found. Install device-tree-compiler (dtc): \
                 brew install dtc / apt install device-tree-compiler"
            )
        });
    if !out.status.success() {
        panic!(
            "dtc failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    println!("cargo:rustc-env=QEMU_VIRT_DTB_PATH={}", dtb.display());
    println!("cargo:rerun-if-changed={}", dts.display());
}
