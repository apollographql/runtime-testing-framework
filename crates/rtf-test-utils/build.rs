fn main() {
    // Force a rebuild if the named files or directories are modified
    println!("cargo::rerun-if-changed=build.rs");
    // println!("cargo::rerun-if-changed=resources");
}
