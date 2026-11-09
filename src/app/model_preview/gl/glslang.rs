//! Compiling GLSL through `glslangValidator`, for the tests that check a
//! program compiles before any context links it.

/// Each `(stage, source)` of `name` through `glslangValidator`; without
/// it, skip by name.
pub(in crate::app::model_preview) fn assert_compiles_under_glslang(name: &str, stages: &[(&str, &str)]) {
    if std::process::Command::new("glslangValidator").arg("--version").output().is_err() {
        eprintln!("skipping: glslangValidator is not on PATH (brew install glslang)");
        return;
    }
    let dir = crate::core::test_kits::unique_temp_dir(&format!("model_preview_{name}_glsl"));
    for (_, source) in stages {
        super::glsl::record_source(source);
    }
    for (stage, source) in stages {
        let path = dir.join(format!("{name}.{stage}"));
        std::fs::write(&path, source).expect("write shader");
        let output = std::process::Command::new("glslangValidator").arg(&path).output().expect("run glslangValidator");
        let printed = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{name} {stage} does not compile:\n{printed}");
    }
}
