//! Versioned, password-encrypted portable backups.
//!
//! A portable backup is a single file that can be copied to removable media or
//! cloud storage without exposing the SQLite database in clear text. The
//! container is deliberately self-describing: algorithm ids, KDF parameters,
//! schema version, salt, nonce and the plaintext hash are stored in an
//! authenticated header. AES-256-GCM protects both confidentiality and
//! integrity; the header is used as associated data so downgrade or parameter
//! tampering fails authentication.

use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use sha2::{Digest, Sha256};

use crate::db::common::CURRENT_SCHEMA_VERSION;

use super::backup::{replace_file, unique_sibling_path};

const MAGIC: &[u8; 8] = b"HESABDBK";
pub(crate) const PORTABLE_EXTENSION: &str = "hacc";
const FORMAT_VERSION: u16 = 1;
const KDF_ARGON2ID: u8 = 1;
const CIPHER_AES_256_GCM: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = 96;
const DEFAULT_M_COST: u32 = 19 * 1024;
const DEFAULT_T_COST: u32 = 2;
const DEFAULT_P_COST: u32 = 1;
const MIN_PASSWORD_CHARS: usize = 8;
const MAX_PASSWORD_BYTES: usize = 1024;
// The whole file is still read into memory for AES-GCM. Keep the cap well
// below the machine's expected RAM until chunked encryption is implemented;
// peak use is roughly the ciphertext plus the decrypted copy.
const MAX_PLAINTEXT_BYTES: u64 = 512 * 1024 * 1024;

/// Rejects passwords that are too weak or unreasonably large before doing any
/// expensive key derivation.
pub(crate) fn validate_portable_password(password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(format!(
            "رمز پشتیبان باید حداقل {MIN_PASSWORD_CHARS} نویسه باشد."
        ));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err("رمز پشتیبان بیش از حد طولانی است.".to_string());
    }
    Ok(())
}

/// Returns true when the file starts with the portable backup magic. The magic
/// itself is not trusted for decryption; the authenticated header is.
pub(crate) fn is_encrypted_backup(path: &Path) -> Result<bool, String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("باز کردن فایل پشتیبان ناموفق بود: {error}"))?;
    let mut magic = [0u8; 8];
    match file.read_exact(&mut magic) {
        Ok(()) => Ok(&magic == MAGIC),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(format!("خواندن فایل پشتیبان ناموفق بود: {error}")),
    }
}

/// Encrypts an already validated SQLite backup into a portable `.hacc` file.
pub(crate) fn encrypt_backup_file(
    source: &Path,
    destination: &Path,
    password: &str,
    schema_version: i64,
) -> Result<(), String> {
    validate_portable_password(password)?;
    let metadata = std::fs::metadata(source)
        .map_err(|error| format!("خواندن اندازهٔ فایل پشتیبان ناموفق بود: {error}"))?;
    if metadata.len() > MAX_PLAINTEXT_BYTES {
        return Err(format!(
            "حجم پایگاه داده برای پشتیبان رمزنگاری‌شده بیش از حد مجاز است (حداکثر {} مگابایت).",
            MAX_PLAINTEXT_BYTES / (1024 * 1024)
        ));
    }
    let plaintext = std::fs::read(source)
        .map_err(|error| format!("خواندن فایل پشتیبان برای رمزنگاری ناموفق بود: {error}"))?;

    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut salt).map_err(|error| format!("تولید salt تصادفی ناموفق بود: {error}"))?;
    getrandom::fill(&mut nonce)
        .map_err(|error| format!("تولید nonce تصادفی ناموفق بود: {error}"))?;

    let header = build_header(
        schema_version,
        DEFAULT_M_COST,
        DEFAULT_T_COST,
        DEFAULT_P_COST,
        &salt,
        &nonce,
        plaintext.len() as u64,
        &sha256(&plaintext),
    )?;
    let mut key = derive_key(
        password,
        &salt,
        DEFAULT_M_COST,
        DEFAULT_T_COST,
        DEFAULT_P_COST,
    )?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| "آماده‌سازی الگوریتم رمزنگاری ناموفق بود.".to_string())?;
    let encrypted = cipher.encrypt(
        Nonce::from_slice(&nonce),
        Payload {
            msg: &plaintext,
            aad: &header,
        },
    );
    key.fill(0);
    let ciphertext = encrypted.map_err(|_| "رمزنگاری فایل پشتیبان ناموفق بود.".to_string())?;

    let mut output = Vec::with_capacity(header.len() + ciphertext.len());
    output.extend_from_slice(&header);
    output.extend_from_slice(&ciphertext);

    let temporary = unique_sibling_path(destination, "portable-tmp");
    let _ = std::fs::remove_file(&temporary);
    let result = std::fs::write(&temporary, &output)
        .map_err(|error| format!("نوشتن فایل پشتیبان رمزنگاری‌شده ناموفق بود: {error}"))
        .and_then(|()| replace_file(&temporary, destination));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Decrypts a portable backup into `destination` after authenticating the
/// header, ciphertext and plaintext hash. The caller is responsible for
/// deleting the temporary plaintext file.
pub(crate) fn decrypt_backup_file(
    source: &Path,
    destination: &Path,
    password: &str,
) -> Result<(), String> {
    let metadata = std::fs::metadata(source)
        .map_err(|error| format!("خواندن اندازهٔ فایل پشتیبان ناموفق بود: {error}"))?;
    if metadata.len() > HEADER_LEN as u64 + MAX_PLAINTEXT_BYTES + TAG_LEN as u64 {
        return Err(format!(
            "حجم فایل پشتیبان رمزنگاری‌شده بیش از حد مجاز است (حداکثر {} مگابایت داده).",
            MAX_PLAINTEXT_BYTES / (1024 * 1024)
        ));
    }
    let bytes = std::fs::read(source)
        .map_err(|error| format!("خواندن فایل پشتیبان رمزنگاری‌شده ناموفق بود: {error}"))?;
    if bytes.len() < HEADER_LEN + TAG_LEN {
        return Err("ساختار فایل پشتیبان رمزنگاری‌شده معتبر نیست.".to_string());
    }
    if &bytes[..8] != MAGIC {
        return Err("فایل انتخاب‌شده پشتیبان رمزنگاری‌شدهٔ این برنامه نیست.".to_string());
    }

    let header = &bytes[..HEADER_LEN];
    let parsed = parse_header(header)?;
    if parsed.version != FORMAT_VERSION {
        return Err(format!(
            "نسخهٔ قالب پشتیبان رمزنگاری‌شده پشتیبانی نمی‌شود: {}",
            parsed.version
        ));
    }
    if parsed.kdf != KDF_ARGON2ID || parsed.cipher != CIPHER_AES_256_GCM {
        return Err("الگوریتم پشتیبان رمزنگاری‌شده پشتیبانی نمی‌شود.".to_string());
    }
    if parsed.plaintext_len > MAX_PLAINTEXT_BYTES {
        return Err("حجم دادهٔ پشتیبان بیش از حد مجاز است.".to_string());
    }
    let expected_len = HEADER_LEN as u64 + parsed.plaintext_len + TAG_LEN as u64;
    if bytes.len() as u64 != expected_len {
        return Err("طول فایل پشتیبان رمزنگاری‌شده با هدر آن هم‌خوانی ندارد.".to_string());
    }
    if !(4..=CURRENT_SCHEMA_VERSION).contains(&parsed.schema_version) {
        return Err(format!(
            "نسخهٔ schema پشتیبان پشتیبانی نمی‌شود: {}",
            parsed.schema_version
        ));
    }

    let mut key = derive_key(
        password,
        &parsed.salt,
        parsed.m_cost,
        parsed.t_cost,
        parsed.p_cost,
    )?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| "آماده‌سازی الگوریتم رمزنگاری ناموفق بود.".to_string())?;
    let plaintext = cipher.decrypt(
        Nonce::from_slice(&parsed.nonce),
        Payload {
            msg: &bytes[HEADER_LEN..],
            aad: header,
        },
    );
    key.fill(0);
    let plaintext = plaintext
        .map_err(|_| "رمز عبور نادرست است یا فایل پشتیبان دستکاری شده است.".to_string())?;

    if plaintext.len() as u64 != parsed.plaintext_len
        || sha256(&plaintext) != parsed.plaintext_sha256
    {
        return Err("صحت دادهٔ رمزگشایی‌شده تأیید نشد.".to_string());
    }

    let temporary = unique_sibling_path(destination, "portable-plain-tmp");
    let _ = std::fs::remove_file(&temporary);
    let result = std::fs::write(&temporary, &plaintext)
        .map_err(|error| format!("نوشتن فایل موقت بازیابی ناموفق بود: {error}"))
        .and_then(|()| replace_file(&temporary, destination));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[derive(Debug)]
struct ParsedHeader {
    version: u16,
    kdf: u8,
    cipher: u8,
    schema_version: i64,
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
    plaintext_len: u64,
    plaintext_sha256: [u8; 32],
}

fn parse_header(header: &[u8]) -> Result<ParsedHeader, String> {
    if header.len() != HEADER_LEN {
        return Err("هدر فایل پشتیبان رمزنگاری‌شده معتبر نیست.".to_string());
    }
    let version = u16::from_le_bytes(header[8..10].try_into().expect("header length checked"));
    let kdf = header[10];
    let cipher = header[11];
    let schema_version = i64::from(u32::from_le_bytes(
        header[12..16].try_into().expect("header length checked"),
    ));
    let m_cost = u32::from_le_bytes(header[16..20].try_into().expect("header length checked"));
    let t_cost = u32::from_le_bytes(header[20..24].try_into().expect("header length checked"));
    let p_cost = u32::from_le_bytes(header[24..28].try_into().expect("header length checked"));
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&header[28..44]);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&header[44..56]);
    let plaintext_len =
        u64::from_le_bytes(header[56..64].try_into().expect("header length checked"));
    let mut plaintext_sha256 = [0u8; 32];
    plaintext_sha256.copy_from_slice(&header[64..96]);

    if !kdf_params_allowed(m_cost, t_cost, p_cost) {
        return Err(
            "پارامترهای KDF در فایل پشتیبان مجاز نیستند؛ فایل ممکن است دست‌کاری شده باشد."
                .to_string(),
        );
    }

    Ok(ParsedHeader {
        version,
        kdf,
        cipher,
        schema_version,
        m_cost,
        t_cost,
        p_cost,
        salt,
        nonce,
        plaintext_len,
        plaintext_sha256,
    })
}

fn kdf_params_allowed(m_cost: u32, t_cost: u32, p_cost: u32) -> bool {
    // The header is authenticated only after key derivation. In format v1 the
    // writer emits exactly one parameter set, so any other value is rejected
    // before Argon2 can allocate memory or consume CPU. A future format version
    // that needs other parameters must add its own explicit allowlist.
    m_cost == DEFAULT_M_COST && t_cost == DEFAULT_T_COST && p_cost == DEFAULT_P_COST
}

#[allow(clippy::too_many_arguments)]
fn build_header(
    schema_version: i64,
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    salt: &[u8; SALT_LEN],
    nonce: &[u8; NONCE_LEN],
    plaintext_len: u64,
    plaintext_sha256: &[u8; 32],
) -> Result<[u8; HEADER_LEN], String> {
    if !kdf_params_allowed(m_cost, t_cost, p_cost) {
        return Err("پارامترهای KDF برای پشتیبان رمزنگاری‌شده مجاز نیستند.".to_string());
    }
    if !(4..=CURRENT_SCHEMA_VERSION).contains(&schema_version) {
        return Err("نسخهٔ schema برای پشتیبان رمزنگاری‌شده معتبر نیست.".to_string());
    }
    let schema_version = u32::try_from(schema_version)
        .map_err(|_| "نسخهٔ schema برای پشتیبان رمزنگاری‌شده معتبر نیست.".to_string())?;
    let mut header = [0u8; HEADER_LEN];
    header[..8].copy_from_slice(MAGIC);
    header[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    header[10] = KDF_ARGON2ID;
    header[11] = CIPHER_AES_256_GCM;
    header[12..16].copy_from_slice(&schema_version.to_le_bytes());
    header[16..20].copy_from_slice(&m_cost.to_le_bytes());
    header[20..24].copy_from_slice(&t_cost.to_le_bytes());
    header[24..28].copy_from_slice(&p_cost.to_le_bytes());
    header[28..44].copy_from_slice(salt);
    header[44..56].copy_from_slice(nonce);
    header[56..64].copy_from_slice(&plaintext_len.to_le_bytes());
    header[64..96].copy_from_slice(plaintext_sha256);
    Ok(header)
}

fn derive_key(
    password: &str,
    salt: &[u8; SALT_LEN],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<[u8; KEY_LEN], String> {
    let params = Params::new(m_cost, t_cost, p_cost, Some(KEY_LEN))
        .map_err(|_| "پارامترهای KDF برای پشتیبان معتبر نیستند.".to_string())?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|_| "محاسبهٔ کلید رمزنگاری ناموفق بود.".to_string())?;
    Ok(key)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_with_params(m_cost: u32, t_cost: u32, p_cost: u32) -> [u8; HEADER_LEN] {
        let mut header = build_header(
            CURRENT_SCHEMA_VERSION,
            DEFAULT_M_COST,
            DEFAULT_T_COST,
            DEFAULT_P_COST,
            &[1u8; SALT_LEN],
            &[2u8; NONCE_LEN],
            8,
            &[3u8; 32],
        )
        .unwrap();
        header[16..20].copy_from_slice(&m_cost.to_le_bytes());
        header[20..24].copy_from_slice(&t_cost.to_le_bytes());
        header[24..28].copy_from_slice(&p_cost.to_le_bytes());
        header
    }

    #[test]
    fn header_accepts_only_the_writer_kdf_allowlist() {
        assert!(parse_header(&header_with_params(
            DEFAULT_M_COST,
            DEFAULT_T_COST,
            DEFAULT_P_COST
        ))
        .is_ok());

        for (m_cost, t_cost, p_cost) in [
            (1024 * 1024, DEFAULT_T_COST, DEFAULT_P_COST),
            (DEFAULT_M_COST, 10, DEFAULT_P_COST),
            (DEFAULT_M_COST, DEFAULT_T_COST, 8),
            (8 * 1024, DEFAULT_T_COST, DEFAULT_P_COST),
        ] {
            let error = parse_header(&header_with_params(m_cost, t_cost, p_cost)).unwrap_err();
            assert!(
                error.contains("KDF"),
                "unexpected error for {m_cost}/{t_cost}/{p_cost}: {error}"
            );
        }
    }

    #[test]
    fn decrypt_rejects_a_tampered_expensive_header_before_key_derivation() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-portable-kdf-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.sqlite3");
        let encrypted = dir.join("backup.hacc");
        let tampered = dir.join("tampered.hacc");
        let plain = dir.join("plain.sqlite3");
        std::fs::write(&source, b"not really a database").unwrap();
        encrypt_backup_file(
            &source,
            &encrypted,
            "correct horse battery",
            CURRENT_SCHEMA_VERSION,
        )
        .unwrap();

        let mut bytes = std::fs::read(&encrypted).unwrap();
        bytes[16..20].copy_from_slice(&(1024 * 1024u32).to_le_bytes());
        std::fs::write(&tampered, bytes).unwrap();

        let error = decrypt_backup_file(&tampered, &plain, "correct horse battery").unwrap_err();
        assert!(error.contains("KDF"), "unexpected error: {error}");
        assert!(!plain.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
