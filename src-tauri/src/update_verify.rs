//! Verification of a downloaded update, failing closed: the file's SHA-256
//! must equal its entry in `SHA256SUMS.txt`, and its minisign signature must
//! verify against the public key built into the app. Nothing here executes
//! anything.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use base64::Engine;
use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};

/// Public key of the update signatures: the base64 of the minisign public key
/// file exactly as `tauri signer generate` writes it.
pub const UPDATE_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEZDQ0JCNkZCNDY2NzIwM0IKUldRN0lHZEcrN2JML0x5VHNxVXNxMVpuY2hHNTdYZTdpWjloNEpIL3JlWDBjSEJHNE9lRjhzYSsK";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("checksum mismatch: {0}")]
    ChecksumMismatch(String),
    #[error("invalid signature: {0}")]
    SignatureInvalid(String),
    #[error("cannot read the file: {0}")]
    Io(String),
}

const CHUNK: usize = 64 * 1024;

fn is_sha256(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `<hash>  <name>` lines (also `<hash> *<name>`), hashes lowercased.
pub fn parse_sha256sums(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_end_matches('\r');
            let (hash, rest) = line.split_once(' ')?;
            let name = rest.trim_start_matches([' ', '*']);
            (is_sha256(hash) && !name.is_empty())
                .then(|| (name.to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

pub fn sha256_file(path: &Path) -> Result<String, VerifyError> {
    let mut file = std::fs::File::open(path).map_err(|e| VerifyError::Io(e.to_string()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| VerifyError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// The file called `name` must hash to its entry in `sums`; no entry fails.
pub fn verify_checksum(path: &Path, name: &str, sums: &str) -> Result<(), VerifyError> {
    let expected = parse_sha256sums(sums)
        .remove(name)
        .ok_or_else(|| VerifyError::ChecksumMismatch(format!("no checksum entry for {name}")))?;
    if sha256_file(path)? == expected {
        Ok(())
    } else {
        Err(VerifyError::ChecksumMismatch(format!(
            "{name} does not match its checksum"
        )))
    }
}

fn decode_text(b64: &str, what: &str) -> Result<String, VerifyError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| VerifyError::SignatureInvalid(format!("the {what} is not valid base64")))?;
    String::from_utf8(bytes)
        .map_err(|_| VerifyError::SignatureInvalid(format!("the {what} is not text")))
}

/// Streams the whole file through the minisign verifier. Both inputs are the
/// base64 form `tauri signer` produces (base64 of the minisign file text).
pub fn verify_signature(
    pubkey_b64: &str,
    signature_b64: &str,
    path: &Path,
) -> Result<(), VerifyError> {
    let invalid = |m: &str| VerifyError::SignatureInvalid(m.to_string());
    let key = PublicKey::decode(&decode_text(pubkey_b64, "public key")?)
        .map_err(|_| invalid("the public key is not valid"))?;
    let signature = Signature::decode(&decode_text(signature_b64, "signature")?)
        .map_err(|_| invalid("the signature is not valid"))?;
    let mut verifier = key
        .verify_stream(&signature)
        .map_err(|_| invalid("the signature does not belong to this key"))?;
    let mut file = std::fs::File::open(path).map_err(|e| VerifyError::Io(e.to_string()))?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| VerifyError::Io(e.to_string()))?;
        if n == 0 {
            break;
        }
        verifier.update(&buf[..n]);
    }
    verifier
        .finalize()
        .map_err(|_| invalid("the file does not match its signature"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const TEST_PUB: &str = include_str!("../tests/fixtures/update/test.pub");
    const OTHER_PUB: &str = include_str!("../tests/fixtures/update/other.pub");
    const SIG: &str = include_str!("../tests/fixtures/update/payload.bin.sig");

    fn payload() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/update/payload.bin")
    }

    fn sha_of_payload() -> String {
        sha256_file(&payload()).unwrap()
    }

    #[test]
    fn the_production_key_is_a_valid_minisign_public_key() {
        use base64::Engine;
        let text = base64::engine::general_purpose::STANDARD
            .decode(UPDATE_PUBKEY)
            .unwrap();
        let text = String::from_utf8(text).unwrap();
        assert!(text.starts_with("untrusted comment: minisign public key"));
        assert!(minisign_verify::PublicKey::decode(&text).is_ok());
    }

    #[test]
    fn a_genuine_signature_verifies_and_every_tamper_fails_closed() {
        verify_signature(TEST_PUB.trim(), SIG.trim(), &payload()).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tampered = tmp.path().join("t.bin");
        let mut bytes = std::fs::read(payload()).unwrap();
        bytes.push(b'!');
        std::fs::write(&tampered, &bytes).unwrap();
        let bad = |pk: &str, sig: &str, f: &Path| {
            matches!(
                verify_signature(pk, sig, f),
                Err(VerifyError::SignatureInvalid(_))
            )
        };
        assert!(bad(TEST_PUB.trim(), SIG.trim(), &tampered), "tampered file");
        bytes.truncate(bytes.len() - 2);
        std::fs::write(&tampered, &bytes).unwrap();
        assert!(
            bad(TEST_PUB.trim(), SIG.trim(), &tampered),
            "truncated file"
        );
        assert!(bad(OTHER_PUB.trim(), SIG.trim(), &payload()), "wrong key");
        let sig = SIG.trim();
        assert!(
            bad(TEST_PUB.trim(), &sig[..sig.len() / 2], &payload()),
            "truncated signature"
        );
        assert!(bad(TEST_PUB.trim(), "", &payload()));
        assert!(bad(TEST_PUB.trim(), "not base64 !!!", &payload()));
        assert!(
            bad(TEST_PUB.trim(), "dW50cnVzdGVkIGNvbW1lbnQ6IHg=", &payload()),
            "decodes but is no signature"
        );
        assert!(bad("", SIG.trim(), &payload()));
        assert!(bad("garbage", SIG.trim(), &payload()));
        assert!(matches!(
            verify_signature(TEST_PUB.trim(), SIG.trim(), &tmp.path().join("missing")),
            Err(VerifyError::Io(_))
        ));
    }

    #[test]
    fn checksum_lines_are_parsed_in_the_common_formats() {
        let h = "a".repeat(64);
        let text = format!(
            "# c\n\n{h}  one.rpm\n{}  *two.deb\r\n{h} three bin\nbadline\n",
            "B".repeat(64)
        );
        let sums = parse_sha256sums(&text);
        assert_eq!(sums.get("one.rpm"), Some(&h));
        assert_eq!(
            sums.get("two.deb"),
            Some(&"b".repeat(64)),
            "hashes are lowercased and a leading * is dropped"
        );
        assert_eq!(sums.get("three bin"), Some(&h));
        assert_eq!(sums.len(), 3);
        assert!(parse_sha256sums("").is_empty());
        assert!(
            parse_sha256sums(&format!("{}  short.bin", "a".repeat(63))).is_empty(),
            "not a SHA-256"
        );
    }

    #[test]
    fn the_hash_of_the_file_must_match_its_own_entry() {
        let h = sha_of_payload();
        assert_eq!(h.len(), 64);
        let good = format!("{}  other.bin\n{h}  payload.bin\n", "0".repeat(64));
        verify_checksum(&payload(), "payload.bin", &good).unwrap();
        verify_checksum(
            &payload(),
            "payload.bin",
            &good.to_uppercase().replace("PAYLOAD.BIN", "payload.bin"),
        )
        .unwrap();
        let wrong = format!("{}  payload.bin\n", "0".repeat(64));
        assert!(matches!(
            verify_checksum(&payload(), "payload.bin", &wrong),
            Err(VerifyError::ChecksumMismatch(_))
        ));
        let missing = format!("{h}  something-else.bin\n");
        let e = verify_checksum(&payload(), "payload.bin", &missing).unwrap_err();
        assert!(
            matches!(&e, VerifyError::ChecksumMismatch(m) if m.contains("no checksum")),
            "{e:?}"
        );
        assert!(matches!(
            verify_checksum(&payload(), "payload.bin", ""),
            Err(VerifyError::ChecksumMismatch(_))
        ));
        let partial_name = format!("{h}  my-payload.bin\n");
        assert!(
            verify_checksum(&payload(), "payload.bin", &partial_name).is_err(),
            "names match exactly"
        );
        assert!(matches!(
            verify_checksum(Path::new("/nope/x"), "payload.bin", &good),
            Err(VerifyError::Io(_))
        ));
    }
}
