use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);
    }

    let mut config = prost_build::Config::new();
    config.type_attribute(".", "#[derive(serde::Serialize, serde::Deserialize)]");
    config.compile_protos(&["proto/rustplus.proto"], &["proto/"])?;

    println!("cargo:rerun-if-changed=proto/");

    Ok(())
}
