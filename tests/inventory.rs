use robot_manager::inventory::{collect, fingerprint, Sha256};
use std::fs;

#[test]
fn sha256_computes_correct_hashes() {
    let mut hasher = Sha256::default();
    hasher.update(b"hello world");
    let result = hasher.finalize();
    let hex = result.iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(
        hex,
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
    );

    let empty_hasher = Sha256::default();
    let empty_result = empty_hasher.finalize();
    let empty_hex = empty_result.iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(
        empty_hex,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn fingerprint_measures_file_length_and_digest() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sample.txt");
    fs::write(&path, b"test content for sha256").unwrap();

    let fp = fingerprint(&path).unwrap();
    assert_eq!(fp.bytes, 23);
    assert_eq!(fp.sha256.len(), 64);
}

#[test]
fn collect_records_sdk_and_runtime_libraries() {
    let temp = tempfile::tempdir().unwrap();
    let sdk = temp.path().join("sdk");
    let runtime = temp.path().join("runtime");

    let headers = sdk.join("usr/local/include/vmxpi");
    let hal_dir = sdk.join("usr/local/lib/vmxpi");
    fs::create_dir_all(&headers).unwrap();
    fs::create_dir_all(&hal_dir).unwrap();
    fs::create_dir_all(&runtime).unwrap();

    fs::write(headers.join("VMXPi.h"), "#pragma once\n").unwrap();
    fs::write(hal_dir.join("libvmxpi_hal_cpp.so"), "binary").unwrap();
    fs::write(runtime.join("libstudica_hardware.so"), "binary").unwrap();
    fs::write(runtime.join("libstudica.so"), "binary").unwrap();
    fs::write(runtime.join("libhalsim_studica.so"), "binary").unwrap();
    fs::write(runtime.join("libwpiHal.so"), "binary").unwrap();

    let record = collect(&sdk, &runtime, "test-os-v1", None, "2027.0.0-w4.beta.1").unwrap();
    assert_eq!(record.schema_version, 1);
    assert_eq!(record.image.id, "test-os-v1");
    assert!(record.missing_required.is_empty(), "missing: {:?}", record.missing_required);
    assert!(record.files.len() >= 6);
}

#[test]
fn collect_identifies_missing_required_libraries() {
    let temp = tempfile::tempdir().unwrap();
    let sdk = temp.path().join("sdk");
    let runtime = temp.path().join("runtime");
    fs::create_dir_all(&sdk).unwrap();
    fs::create_dir_all(&runtime).unwrap();

    let record = collect(&sdk, &runtime, "empty-image", None, "beta").unwrap();
    assert_eq!(record.missing_required.len(), 6);
}

/// Ported from the retired Python inventory tests: an incomplete deploy is
/// explicit, and a changed binary changes the recorded identity.
#[test]
fn missing_files_are_explicit_and_changes_change_identity() {
    let temp = tempfile::tempdir().unwrap();
    let sdk = temp.path().join("sdk");
    let runtime = temp.path().join("runtime");
    fs::create_dir_all(&runtime).unwrap();
    let library = runtime.join("libhalsim_studica.so");
    fs::write(&library, b"first binary").unwrap();

    let first = collect(&sdk, &runtime, "image", None, "release").unwrap();
    assert!(first.image.sha256.is_none());
    assert!(first
        .missing_required
        .iter()
        .any(|item| item.contains("libwpiHal.so") && item.contains("libwpiHald.so")));
    assert!(!first.missing_required.iter().any(|item| item.contains("libhalsim_studica.so")));

    fs::write(&library, b"second binary").unwrap();
    let second = collect(&sdk, &runtime, "image", None, "release").unwrap();
    let before = &first.files[&library.to_string_lossy().to_string()].sha256;
    let after = &second.files[&library.to_string_lossy().to_string()].sha256;
    assert_ne!(before, after, "a changed binary must change the recorded hash");
}

#[test]
fn either_build_type_without_separate_drivers_is_complete() {
    let temp = tempfile::tempdir().unwrap();
    let sdk = temp.path().join("sdk");
    let runtime = temp.path().join("runtime");
    fs::create_dir_all(sdk.join("usr/local/include/vmxpi")).unwrap();
    fs::create_dir_all(sdk.join("usr/local/lib/vmxpi")).unwrap();
    fs::create_dir_all(&runtime).unwrap();
    fs::write(sdk.join("usr/local/include/vmxpi/VMXPi.h"), "//").unwrap();
    fs::write(sdk.join("usr/local/lib/vmxpi/libvmxpi_hal_cpp.so"), "hal").unwrap();
    for name in [
        "libhalsim_studicad.so",
        "libstudica_hardware.so",
        "libstudica.so",
        "libwpiHald.so",
    ] {
        fs::write(runtime.join(name), "x").unwrap();
    }

    let record = collect(&sdk, &runtime, "image", None, "release").unwrap();
    assert!(record.missing_required.is_empty(), "{:?}", record.missing_required);
}
