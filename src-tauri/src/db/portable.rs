//! Versioned, password-encrypted portable backups.
//!
//! A portable backup is a single file that can be copied to removable media or
//! cloud storage without exposing the SQLite database in clear text. The
//! container is deliberately self-describing: algorithm ids, KDF parameters,
//! schema version, salt, nonce and the plaintext hash are stored in an
//! authenticated header. AES-256-GCM protects both confidentiality and
//! integrity; the header is used as associated data so downgrade or parameter
//! tampering fails authentication.
//!
//! Format v2 encrypts the SQLite file in independent 4 MiB chunks. Only one
//! plaintext chunk and its ciphertext are in memory at once, so a 512 MiB
//! backup no longer needs a gigabyte of RAM. Format v1 files are still read
//! with the original whole-file path.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use sha2::{Digest, Sha256};

use crate::db::common::CURRENT_SCHEMA_VERSION;

use super::backup::{create_private_file, replace_file, unique_sibling_path, PlaintextTempFile};

const MAGIC: &[u8; 8] = b"HESABDBK";
pub(crate) const PORTABLE_EXTENSION: &str = "hacc";
const FORMAT_VERSION_V1: u16 = 1;
const FORMAT_VERSION: u16 = 2;
const KDF_ARGON2ID: u8 = 1;
const CIPHER_AES_256_GCM: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = 96;
const CHUNK_LEN: usize = 4 * 1024 * 1024;
const CHUNK_FRAME_LEN: usize = 4 + TAG_LEN;
const DEFAULT_M_COST: u32 = 19 * 1024;
const DEFAULT_T_COST: u32 = 2;
const DEFAULT_P_COST: u32 = 1;
const MIN_PASSWORD_CHARS: usize = 8;
const MAX_PASSWORD_BYTES: usize = 1024;
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
        .map_err(|error| format!("خواندن اطلاعات فایل پشتیبان ناموفق بود: {error}"))?;
    if metadata.len() > MAX_PLAINTEXT_BYTES {
        return Err(format!(
            "این حجم پشتیبان برای رمزنگاری بیش از حد مجاز است (حداکثر {} مگابایت).",
            MAX_PLAINTEXT_BYTES / (1024 * 1024)
        ));
    }

    // Pass 1: hash and measure the plaintext without keeping it in memory. The
    // header commits to both values and is authenticated with every chunk.
    let (plaintext_len, plaintext_sha256) = hash_file(source)?;
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut salt).map_err(|error| format!("تولید salt تصادفی ناموفق بود: {error}"))?;
    // The last four nonce bytes are the chunk index, so only the first eight
    // are random. They stay unused/zero in the header for v2.
    getrandom::fill(&mut nonce[..8])
        .map_err(|error| format!("تولید nonce تصادفی ناموفق بود: {error}"))?;

    let header = build_header(
        schema_version,
        DEFAULT_M_COST,
        DEFAULT_T_COST,
        DEFAULT_P_COST,
        &salt,
        &nonce,
        plaintext_len,
        &plaintext_sha256,
    )?;
    let mut key = derive_key(
        password,
        &salt,
        DEFAULT_M_COST,
        DEFAULT_T_COST,
        DEFAULT_P_COST,
    )?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| "ساخت رمزکننده پشتیبان ناموفق بود.".to_string())?;

    let temporary = PlaintextTempFile::new(unique_sibling_path(destination, "portable-tmp"));
    let result = (|| -> Result<(), String> {
        let mut input = std::fs::File::open(source)
            .map_err(|error| format!("باز کردن فایل پشتیبان ناموفق بود: {error}"))?;
        input
            .seek(SeekFrom::Start(0))
            .map_err(|error| format!("خواندن فایل پشتیبان ناموفق بود: {error}"))?;
        let mut output = create_private_file(temporary.path())?;
        output
            .write_all(&header)
            .map_err(|error| format!("نوشتن فایل پشتیبان ناموفق بود: {error}"))?;

        let mut buffer = vec![0u8; CHUNK_LEN];
        let mut remaining = plaintext_len;
        let mut index = 0u32;
        while remaining > 0 || index == 0 {
            let plaintext_chunk_len = remaining.min(CHUNK_LEN as u64) as usize;
            if plaintext_chunk_len > 0 {
                input
                    .read_exact(&mut buffer[..plaintext_chunk_len])
                    .map_err(|error| {
                        format!("خواندن فایل پشتیبان در حین رمزنگاری ناموفق بود: {error}")
                    })?;
            }
            let aad = chunk_aad(&header, index, plaintext_chunk_len);
            let nonce = chunk_nonce(&nonce, index);
            let encrypted = cipher
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &buffer[..plaintext_chunk_len],
                        aad: &aad,
                    },
                )
                .map_err(|_| "رمزنگاری فایل پشتیبان ناموفق بود.".to_string())?;
            output
                .write_all(&(plaintext_chunk_len as u32).to_le_bytes())
                .map_err(|error| format!("نوشتن فایل پشتیبان ناموفق بود: {error}"))?;
            output
                .write_all(&encrypted)
                .map_err(|error| format!("نوشتن فایل پشتیبان ناموفق بود: {error}"))?;
            remaining -= plaintext_chunk_len as u64;
            index = index
                .checked_add(1)
                .ok_or_else(|| "تعداد قطعات پشتیبان بیش از حد مجاز است.".to_string())?;
        }
        output
            .sync_all()
            .map_err(|error| format!("تکمیل نوشتن فایل پشتیبان ناموفق بود: {error}"))?;
        Ok(())
    })();
    key.fill(0);

    result?;
    replace_file(temporary.path(), destination)
}

/// Decrypts a portable backup into `destination` after authenticating the
/// header and every chunk. The caller is responsible for deleting the temporary
/// plaintext file when it is no longer needed.
pub(crate) fn decrypt_backup_file(
    source: &Path,
    destination: &Path,
    password: &str,
) -> Result<(), String> {
    let metadata = std::fs::metadata(source)
        .map_err(|error| format!("خواندن اطلاعات فایل پشتیبان ناموفق بود: {error}"))?;
    let max_file_len = HEADER_LEN as u64
        + MAX_PLAINTEXT_BYTES
        + max_chunks(MAX_PLAINTEXT_BYTES) as u64 * CHUNK_FRAME_LEN as u64;
    if metadata.len() > max_file_len {
        return Err(format!(
            "این فایل پشتیبان رمزنگاری‌شده بیش از حد مجاز است (حداکثر {} مگابایت داده).",
            MAX_PLAINTEXT_BYTES / (1024 * 1024)
        ));
    }

    let mut file = std::fs::File::open(source)
        .map_err(|error| format!("خواندن فایل پشتیبان رمزنگاری‌شده ناموفق بود: {error}"))?;
    let mut header = [0u8; HEADER_LEN];
    file.read_exact(&mut header)
        .map_err(|_| "ساختار فایل پشتیبان رمزنگاری‌شده معتبر نیست.".to_string())?;
    if &header[..8] != MAGIC {
        return Err("فایل انتخاب‌شده یک پشتیبان رمزنگاری‌شده نیست.".to_string());
    }
    let parsed = parse_header(&header)?;
    if parsed.kdf != KDF_ARGON2ID || parsed.cipher != CIPHER_AES_256_GCM {
        return Err("الگوریتم فایل پشتیبان پشتیبانی نمی‌شود.".to_string());
    }
    if parsed.plaintext_len > MAX_PLAINTEXT_BYTES {
        return Err("حجم داده پشتیبان بیش از حد مجاز است.".to_string());
    }
    if !(4..=CURRENT_SCHEMA_VERSION).contains(&parsed.schema_version) {
        return Err(format!(
            "نسخه schema پشتیبان پشتیبانی نمی‌شود: {}",
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
    let result = match parsed.version {
        FORMAT_VERSION_V1 => {
            decrypt_v1(source, destination, &header, &parsed, &key, metadata.len())
        }
        FORMAT_VERSION => decrypt_v2(
            source,
            destination,
            &header,
            &parsed,
            &key,
            metadata.len(),
            &mut file,
        ),
        version => Err(format!("نسخه فایل پشتیبان پشتیبانی نمی‌شود: {version}")),
    };
    key.fill(0);
    result
}

fn decrypt_v1(
    source: &Path,
    destination: &Path,
    header: &[u8; HEADER_LEN],
    parsed: &ParsedHeader,
    key: &[u8; KEY_LEN],
    file_len: u64,
) -> Result<(), String> {
    let expected_len = HEADER_LEN as u64 + parsed.plaintext_len + TAG_LEN as u64;
    if file_len != expected_len {
        return Err("حجم فایل پشتیبان رمزنگاری‌شده با سربرگ آن یکسان نیست.".to_string());
    }
    let bytes = std::fs::read(source)
        .map_err(|error| format!("خواندن فایل پشتیبان رمزنگاری‌شده ناموفق بود: {error}"))?;
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| "ساخت رمزگشا ناموفق بود.".to_string())?;
    let plaintext = cipher.decrypt(
        Nonce::from_slice(&parsed.nonce),
        Payload {
            msg: &bytes[HEADER_LEN..],
            aad: header,
        },
    );
    let plaintext =
        plaintext.map_err(|_| "رمز عبور نادرست است یا فایل پشتیبان آسیب دیده است.".to_string())?;
    if plaintext.len() as u64 != parsed.plaintext_len
        || sha256(&plaintext) != parsed.plaintext_sha256
    {
        return Err("محتوای رمزگشایی‌شده معتبر نیست.".to_string());
    }
    write_plaintext(destination, |output| {
        output
            .write_all(&plaintext)
            .map_err(|error| format!("نوشتن فایل بازیابی‌شده ناموفق بود: {error}"))
    })
}

fn decrypt_v2(
    _source: &Path,
    destination: &Path,
    header: &[u8; HEADER_LEN],
    parsed: &ParsedHeader,
    key: &[u8; KEY_LEN],
    file_len: u64,
    file: &mut std::fs::File,
) -> Result<(), String> {
    let chunks = max_chunks(parsed.plaintext_len);
    let expected_len =
        HEADER_LEN as u64 + parsed.plaintext_len + chunks as u64 * CHUNK_FRAME_LEN as u64;
    if file_len != expected_len {
        return Err("حجم فایل پشتیبان رمزنگاری‌شده با سربرگ آن یکسان نیست.".to_string());
    }
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| "ساخت رمزگشا ناموفق بود.".to_string())?;

    write_plaintext(destination, |output| {
        let mut hasher = Sha256::new();
        let mut remaining = parsed.plaintext_len;
        let mut index = 0u32;
        let mut total = 0u64;
        while remaining > 0 || index == 0 {
            let mut length_bytes = [0u8; 4];
            file.read_exact(&mut length_bytes)
                .map_err(|_| "ساختار فایل پشتیبان رمزنگاری‌شده معتبر نیست.".to_string())?;
            let chunk_len = u32::from_le_bytes(length_bytes) as usize;
            if chunk_len > CHUNK_LEN || chunk_len as u64 > remaining {
                return Err("ساختار قطعات فایل پشتیبان معتبر نیست.".to_string());
            }
            let mut encrypted = vec![0u8; chunk_len + TAG_LEN];
            file.read_exact(&mut encrypted)
                .map_err(|_| "ساختار فایل پشتیبان رمزنگاری‌شده معتبر نیست.".to_string())?;
            let aad = chunk_aad(header, index, chunk_len);
            let nonce = chunk_nonce(&parsed.nonce, index);
            let plaintext = cipher
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &encrypted,
                        aad: &aad,
                    },
                )
                .map_err(|_| "رمز عبور نادرست است یا فایل پشتیبان آسیب دیده است.".to_string())?;
            output
                .write_all(&plaintext)
                .map_err(|error| format!("نوشتن فایل بازیابی‌شده ناموفق بود: {error}"))?;
            hasher.update(&plaintext);
            total += chunk_len as u64;
            remaining -= chunk_len as u64;
            index = index
                .checked_add(1)
                .ok_or_else(|| "تعداد قطعات پشتیبان بیش از حد مجاز است.".to_string())?;
        }
        let mut trailing = [0u8; 1];
        if file.read(&mut trailing).unwrap_or(0) != 0 {
            return Err("فایل پشتیبان داده اضافی پس از پایان محتوا دارد.".to_string());
        }
        if total != parsed.plaintext_len || hasher.finalize().as_slice() != parsed.plaintext_sha256
        {
            return Err("محتوای رمزگشایی‌شده معتبر نیست.".to_string());
        }
        Ok(())
    })
}

fn write_plaintext(
    destination: &Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<(), String>,
) -> Result<(), String> {
    let temporary = PlaintextTempFile::new(unique_sibling_path(destination, "portable-plain-tmp"));
    let result: Result<(), String> = (|| {
        let mut output = create_private_file(temporary.path())?;
        write(&mut output)?;
        output
            .sync_all()
            .map_err(|error| format!("تکمیل نوشتن فایل بازیابی‌شده ناموفق بود: {error}"))?;
        Ok(())
    })();
    result?;
    replace_file(temporary.path(), destination)
}

fn max_chunks(plaintext_len: u64) -> usize {
    let chunks = plaintext_len.div_ceil(CHUNK_LEN as u64);
    chunks.max(1) as usize
}

fn chunk_nonce(base: &[u8; NONCE_LEN], index: u32) -> [u8; NONCE_LEN] {
    let mut nonce = *base;
    nonce[8..12].copy_from_slice(&index.to_le_bytes());
    nonce
}

fn chunk_aad(header: &[u8; HEADER_LEN], index: u32, plaintext_len: usize) -> Vec<u8> {
    let mut aad = Vec::with_capacity(HEADER_LEN + 8);
    aad.extend_from_slice(header);
    aad.extend_from_slice(&index.to_le_bytes());
    aad.extend_from_slice(&(plaintext_len as u32).to_le_bytes());
    aad
}

fn hash_file(path: &Path) -> Result<(u64, [u8; 32]), String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("باز کردن فایل پشتیبان ناموفق بود: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("خواندن فایل پشتیبان ناموفق بود: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((total, hasher.finalize().into()))
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
        return Err("سربرگ فایل پشتیبان رمزنگاری‌شده معتبر نیست.".to_string());
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
            "پارامترهای KDF در فایل پشتیبان مجاز نیستند؛ فایل ممکن است دستکاری شده باشد."
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
    // The header is authenticated only after key derivation. In formats v1 and
    // v2 the writer emits exactly one parameter set, so any other value is
    // rejected before Argon2 can allocate memory or consume CPU. A future
    // format version that needs other parameters must add its own allowlist.
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
        return Err("پارامترهای KDF فایل پشتیبان مجاز نیستند.".to_string());
    }
    if !(4..=CURRENT_SCHEMA_VERSION).contains(&schema_version) {
        return Err("نسخه schema فایل پشتیبان معتبر نیست.".to_string());
    }
    let schema_version = u32::try_from(schema_version)
        .map_err(|_| "نسخه schema فایل پشتیبان معتبر نیست.".to_string())?;
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
        .map_err(|_| "پارامترهای KDF فایل پشتیبان معتبر نیستند.".to_string())?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|_| "تولید کلید رمزنگاری ناموفق بود.".to_string())?;
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

    #[test]
    fn v2_roundtrip_handles_multiple_chunks_and_rejects_wrong_password() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hesabdari-portable-v2-{stamp}"));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.sqlite3");
        let encrypted = dir.join("backup.hacc");
        let plain = dir.join("plain.sqlite3");
        // Slightly larger than one chunk so the framing loop is exercised.
        let mut bytes = vec![0u8; CHUNK_LEN + 37];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }
        std::fs::write(&source, &bytes).unwrap();

        encrypt_backup_file(
            &source,
            &encrypted,
            "correct horse battery",
            CURRENT_SCHEMA_VERSION,
        )
        .unwrap();
        let encoded = std::fs::read(&encrypted).unwrap();
        assert_eq!(
            u16::from_le_bytes(encoded[8..10].try_into().unwrap()),
            FORMAT_VERSION
        );

        let error = decrypt_backup_file(&encrypted, &plain, "wrong horse battery").unwrap_err();
        assert!(error.contains("رمز"), "unexpected error: {error}");
        assert!(!plain.exists());

        decrypt_backup_file(&encrypted, &plain, "correct horse battery").unwrap();
        assert_eq!(std::fs::read(&plain).unwrap(), bytes);

        // A flipped ciphertext byte must be detected by the chunk tag.
        let mut tampered = encoded;
        tampered[HEADER_LEN + 40] ^= 0x40;
        std::fs::write(&encrypted, tampered).unwrap();
        let error = decrypt_backup_file(&encrypted, &plain, "correct horse battery").unwrap_err();
        assert!(
            error.contains("رمز") || error.contains("معتبر"),
            "unexpected error: {error}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
