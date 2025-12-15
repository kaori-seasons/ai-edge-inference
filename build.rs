fn main() {
    // 构建脚本
    println!("cargo:rerun-if-changed=build.rs");
}
