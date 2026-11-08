use std::{collections::HashMap, env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=SEMMELTV_FONT_DIR");
    let fonts = env::var("SEMMELTV_FONT_DIR").expect("SEMMELTV_FONT_DIR must point at Inter");
    let config = slint_build::CompilerConfiguration::new()
        .with_library_paths(HashMap::from([("fonts".to_string(), PathBuf::from(fonts))]));
    slint_build::compile_with_config("ui/app.slint", config).unwrap();
}
