use std::env;
use std::fs;
use std::path::PathBuf;

const FORK_STAMP: &str = "Heurex fork";
const COMPANY_NAME: &str = "Heurex";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    let parts = VersionParts::parse(&version).unwrap_or(VersionParts { major: 0, minor: 0, patch: 0, build: 0 });
    let resource = build_res_file(&build_version_info(&version, parts));

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let res_path = out_dir.join("sentrux-version.res");
    fs::write(&res_path, resource).expect("write Windows version resource");

    println!("cargo:rustc-link-arg-bin=sentrux={}", res_path.display());
}

#[derive(Clone, Copy)]
struct VersionParts {
    major: u16,
    minor: u16,
    patch: u16,
    build: u16,
}

impl VersionParts {
    fn parse(version: &str) -> Option<Self> {
        let numeric = version.split(['-', '+']).next()?;
        let mut parts = numeric.split('.');
        Some(Self {
            major: parts.next()?.parse().ok()?,
            minor: parts.next()?.parse().ok()?,
            patch: parts.next()?.parse().ok()?,
            build: parts.next().unwrap_or("0").parse().ok()?,
        })
    }

    fn ms(self) -> u32 {
        ((self.major as u32) << 16) | self.minor as u32
    }

    fn ls(self) -> u32 {
        ((self.patch as u32) << 16) | self.build as u32
    }

    fn dotted(self) -> String {
        format!("{}.{}.{}.{}", self.major, self.minor, self.patch, self.build)
    }
}

fn build_version_info(version: &str, parts: VersionParts) -> Vec<u8> {
    let mut buf = Vec::new();

    let root = start_block(&mut buf, "VS_VERSION_INFO", 52, 0);
    push_fixed_file_info(&mut buf, parts);
    align4(&mut buf);

    let strings = start_block(&mut buf, "StringFileInfo", 0, 1);
    let table = start_block(&mut buf, "040904B0", 0, 1);
    let file_version = format!("{} ({})", parts.dotted(), FORK_STAMP);
    let product_version = format!("{version}-heurex-fork");
    add_string(&mut buf, "CompanyName", COMPANY_NAME);
    add_string(&mut buf, "FileDescription", "Sentrux Heurex fork");
    add_string(&mut buf, "FileVersion", &file_version);
    add_string(&mut buf, "InternalName", "sentrux");
    add_string(&mut buf, "OriginalFilename", "sentrux.exe");
    add_string(&mut buf, "ProductName", "Sentrux Heurex fork");
    add_string(&mut buf, "ProductVersion", &product_version);
    add_string(&mut buf, "PrivateBuild", FORK_STAMP);
    add_string(&mut buf, "Comments", "Heurex fork build. Not the official upstream Sentrux release.");
    end_block(&mut buf, table);
    end_block(&mut buf, strings);

    let vars = start_block(&mut buf, "VarFileInfo", 0, 1);
    let translation = start_block(&mut buf, "Translation", 4, 0);
    push_u16(&mut buf, 0x0409);
    push_u16(&mut buf, 1200);
    align4(&mut buf);
    end_block(&mut buf, translation);
    end_block(&mut buf, vars);

    align4(&mut buf);
    end_block(&mut buf, root);
    buf
}

fn push_fixed_file_info(buf: &mut Vec<u8>, parts: VersionParts) {
    push_u32(buf, 0xFEEF04BD);
    push_u32(buf, 0x00010000);
    push_u32(buf, parts.ms());
    push_u32(buf, parts.ls());
    push_u32(buf, parts.ms());
    push_u32(buf, parts.ls());
    push_u32(buf, 0x0000003F);
    push_u32(buf, 0x00000008);
    push_u32(buf, 0x00040004);
    push_u32(buf, 0x00000001);
    push_u32(buf, 0);
    push_u32(buf, 0);
    push_u32(buf, 0);
}

fn build_res_file(version_info: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();

    push_resource_header(&mut buf, 0, 0, 0, 0, 0);
    push_resource_header(&mut buf, version_info.len() as u32, 16, 1, 0x0030, 0x0409);
    buf.extend_from_slice(version_info);
    align4(&mut buf);

    buf
}

fn push_resource_header(
    buf: &mut Vec<u8>,
    data_size: u32,
    resource_type: u16,
    name: u16,
    memory_flags: u16,
    language: u16,
) {
    push_u32(buf, data_size);
    push_u32(buf, 32);
    push_u16(buf, 0xFFFF);
    push_u16(buf, resource_type);
    push_u16(buf, 0xFFFF);
    push_u16(buf, name);
    push_u32(buf, 0);
    push_u16(buf, memory_flags);
    push_u16(buf, language);
    push_u32(buf, 0);
    push_u32(buf, 0);
    align4(buf);
}

fn add_string(buf: &mut Vec<u8>, key: &str, value: &str) {
    let block = start_block(buf, key, (value.encode_utf16().count() + 1) as u16, 1);
    push_wide_z(buf, value);
    align4(buf);
    end_block(buf, block);
}

fn start_block(buf: &mut Vec<u8>, key: &str, value_len: u16, value_type: u16) -> usize {
    let start = buf.len();
    push_u16(buf, 0);
    push_u16(buf, value_len);
    push_u16(buf, value_type);
    push_wide_z(buf, key);
    align4(buf);
    start
}

fn end_block(buf: &mut [u8], start: usize) {
    let len = buf.len() - start;
    assert!(len <= u16::MAX as usize, "VERSIONINFO block too large");
    let len = (len as u16).to_le_bytes();
    buf[start] = len[0];
    buf[start + 1] = len[1];
}

fn push_wide_z(buf: &mut Vec<u8>, value: &str) {
    for unit in value.encode_utf16() {
        push_u16(buf, unit);
    }
    push_u16(buf, 0);
}

fn align4(buf: &mut Vec<u8>) {
    while buf.len() % 4 != 0 {
        buf.push(0);
    }
}

fn push_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}
