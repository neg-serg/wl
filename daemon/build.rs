use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let shader_out = Path::new(&out_dir).join("shaders");
    fs::create_dir_all(&shader_out).expect("Failed to create shader output directory");

    let shader_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("No parent directory")
        .join("shaders");

    println!("cargo:rerun-if-changed={}", shader_dir.display());

    let entries = fs::read_dir(&shader_dir).expect("Failed to read shaders directory");

    for entry in entries {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "vert" && ext != "frag" && ext != "comp" {
            continue;
        }

        println!("cargo:rerun-if-changed={}", path.display());

        let stem = path.file_name().unwrap().to_str().unwrap();
        let spv_name = format!("{stem}.spv");
        let spv_path = shader_out.join(&spv_name);

        let status = Command::new("glslc")
            .arg(&path)
            .arg("-o")
            .arg(&spv_path)
            .status()
            .expect("Failed to run glslc. Is the Vulkan SDK installed?");

        if !status.success() {
            panic!("glslc failed to compile {}", path.display());
        }
    }
}
