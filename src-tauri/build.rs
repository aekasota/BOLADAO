use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct VendorEntry {
    version: String,
    sha256: String,
}

fn collect_files_sorted(root: &Path, cur: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(cur)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let p = entry.path();
        if p.is_dir() {
            collect_files_sorted(root, &p, out)?;
        } else if p.is_file() {
            out.push(p.strip_prefix(root).unwrap().to_path_buf());
        }
    }
    Ok(())
}

fn pack_component(
    key: &str,
    src_dir: &Path,
    entry: &VendorEntry,
    out_dir: &Path,
    threads: u32,
) -> (usize, u64, String, PathBuf) {
    if !src_dir.exists() {
        panic!(
            "Diretório vendor/bin/{key} não encontrado em {}. Rode `powershell -File scripts/fetch-binaries.ps1` antes de compilar.",
            src_dir.display()
        );
    }

    let mut rel_files = Vec::new();
    collect_files_sorted(src_dir, src_dir, &mut rel_files)
        .unwrap_or_else(|e| panic!("Falha ao listar {}: {e}", src_dir.display()));

    let mut total_bytes = 0u64;
    for rel in &rel_files {
        let meta = fs::metadata(src_dir.join(rel)).unwrap();
        total_bytes += meta.len();
    }
    let file_count = rel_files.len();

    let archive_path = out_dir.join(format!("{key}.tar.zst"));
    let stamp_path = out_dir.join(format!("{key}.stamp"));
    let expected_stamp = format!("{}:{}:{}:{}", entry.version, entry.sha256, file_count, total_bytes);

    let up_to_date = archive_path.exists()
        && stamp_path.exists()
        && fs::read_to_string(&stamp_path)
            .map(|s| s.trim() == expected_stamp)
            .unwrap_or(false);

    if !up_to_date {
        let tmp_archive = out_dir.join(format!("{key}.tar.zst.tmp"));
        {
            let file = File::create(&tmp_archive)
                .unwrap_or_else(|e| panic!("Erro criando {}: {e}", tmp_archive.display()));
            let buf = BufWriter::with_capacity(1024 * 1024, file);
            let mut zstd_enc = zstd::stream::write::Encoder::new(buf, 19)
                .expect("Falha iniciando encoder zstd nível 19");
            let _ = zstd_enc.multithread(threads);
            {
                let mut tar_builder = tar::Builder::new(zstd_enc);
                for rel in &rel_files {
                    let full = src_dir.join(rel);
                    // Normalize tar internal paths to forward slashes
                    let tar_rel = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    tar_builder
                        .append_path_with_name(&full, &tar_rel)
                        .unwrap_or_else(|e| panic!("Erro adicionando {tar_rel} ao tar: {e}"));
                }
                let zstd_enc = tar_builder.into_inner().expect("Erro finalizando tar");
                let mut buf = zstd_enc.finish().expect("Erro finalizando compressão zstd");
                buf.flush().unwrap();
            }
        }
        if archive_path.exists() {
            let _ = fs::remove_file(&archive_path);
        }
        fs::rename(&tmp_archive, &archive_path).expect("Erro movendo arquivo tar.zst final");
        fs::write(&stamp_path, &expected_stamp).expect("Erro gravando stamp");
    }

    // Compute SHA-256 of the compressed archive (cached in .sha256 alongside stamp)
    let sha_cache_path = out_dir.join(format!("{key}.archive_sha256"));
    let archive_sha = if up_to_date && sha_cache_path.exists() {
        fs::read_to_string(&sha_cache_path).unwrap_or_default().trim().to_string()
    } else {
        let mut hasher = Sha256::new();
        let mut f = File::open(&archive_path).unwrap();
        let mut chunk = vec![0u8; 256 * 1024];
        loop {
            let n = f.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            hasher.update(&chunk[..n]);
        }
        let hex_digest = hex::encode(hasher.finalize());
        let _ = fs::write(&sha_cache_path, &hex_digest);
        hex_digest
    };

    (file_count, total_bytes, archive_sha, archive_path)
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest_dir.parent().unwrap();
    let vendor_manifest_path = repo_root.join("vendor").join("manifest.json");
    let vendor_bin_dir = repo_root.join("vendor").join("bin");

    println!("cargo:rerun-if-changed={}", vendor_manifest_path.display());
    println!("cargo:rerun-if-changed={}", vendor_bin_dir.join("ytdlp").join(".version").display());
    println!("cargo:rerun-if-changed={}", vendor_bin_dir.join("deno").join(".version").display());
    println!("cargo:rerun-if-changed={}", vendor_bin_dir.join("ffmpeg").join(".version").display());

    let raw_manifest = fs::read_to_string(&vendor_manifest_path).unwrap_or_else(|e| {
        panic!(
            "Não foi possível ler {}: {e}",
            vendor_manifest_path.display()
        )
    });
    let entries: BTreeMap<String, VendorEntry> =
        serde_json::from_str(&raw_manifest).expect("vendor/manifest.json inválido");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);

    let mut generated = String::new();
    generated.push_str("// Auto-generated by build.rs — do not edit.\n");
    generated.push_str("#[derive(Debug, Clone, Copy, serde::Serialize)]\n");
    generated.push_str("pub struct EmbeddedComponentMeta {\n");
    generated.push_str("    pub key: &'static str,\n");
    generated.push_str("    pub version: &'static str,\n");
    generated.push_str("    pub source_sha256: &'static str,\n");
    generated.push_str("    pub archive_sha256: &'static str,\n");
    generated.push_str("    pub file_count: usize,\n");
    generated.push_str("    pub total_bytes: u64,\n");
    generated.push_str("    pub compressed_bytes: u64,\n");
    generated.push_str("}\n\n");

    let mut const_list = Vec::new();
    for key in ["ytdlp", "deno", "ffmpeg"] {
        let entry = entries
            .get(key)
            .unwrap_or_else(|| panic!("Componente {key} ausente em vendor/manifest.json"));
        let src_dir = vendor_bin_dir.join(key);
        let (file_count, total_bytes, archive_sha, archive_path) =
            pack_component(key, &src_dir, entry, &out_dir, threads);
        let compressed_bytes = fs::metadata(&archive_path).unwrap().len();
        let upper = key.to_uppercase();
        generated.push_str(&format!(
            "pub const EMBEDDED_{upper}_META: EmbeddedComponentMeta = EmbeddedComponentMeta {{\n    key: \"{key}\",\n    version: \"{}\",\n    source_sha256: \"{}\",\n    archive_sha256: \"{archive_sha}\",\n    file_count: {file_count},\n    total_bytes: {total_bytes},\n    compressed_bytes: {compressed_bytes},\n}};\n\n",
            entry.version, entry.sha256
        ));
        const_list.push(format!("EMBEDDED_{upper}_META"));
    }

    generated.push_str(&format!(
        "pub const EMBEDDED_MANIFEST: [EmbeddedComponentMeta; 3] = [{}];\n",
        const_list.join(", ")
    ));

    fs::write(out_dir.join("embedded_manifest.rs"), generated)
        .expect("Falha escrevendo embedded_manifest.rs");

    tauri_build::build();
}
