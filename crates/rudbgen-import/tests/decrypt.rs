//! jdbgen's two encryption schemes, checked against ciphertext jdbgen wrote.
//!
//! `tests/vectors/decrypt.json` was produced by jdbgen's own `StrUtils` and by
//! the `legacyEncrypt` helper of its `EncryptionTest`; the file's `_source` key
//! records how. jdbgen's suite fixes no ciphertext of its own — every value in
//! it is encrypted and decrypted inside one run — so the constants had to come
//! from the implementation rather than be copied out of the tests.
//!
//! One [`Decryptor`] is shared across the whole v2 table on purpose: the
//! vectors share a session salt, and the key cache is what keeps 210,000
//! iterations of PBKDF2 from being paid eight times over.

use rudbgen_import::{Decryptor, Error};
use serde::Deserialize;

/// The vector table as it is on disk.
#[derive(Debug, Deserialize)]
struct Vectors {
    master: String,
    wrong_master: String,
    values: Vec<Vector>,
}

/// One plain text and the two envelopes jdbgen writes it in.
#[derive(Debug, Deserialize)]
struct Vector {
    plain: String,
    v2: String,
    legacy: String,
}

fn vectors() -> Vectors {
    let raw = include_str!("vectors/decrypt.json");
    serde_json::from_str(raw).expect("the checked-in vectors are valid JSON")
}

#[test]
fn every_current_format_vector_decrypts_to_what_jdbgen_encrypted() {
    let vectors = vectors();
    let decryptor = Decryptor::new(&vectors.master);
    for vector in &vectors.values {
        let opened = decryptor
            .decrypt(&vector.v2, "userPassword")
            .unwrap_or_else(|error| panic!("{:?}: {error}", vector.plain));
        assert_eq!(opened, vector.plain);
    }
}

#[test]
fn every_superseded_format_vector_decrypts_to_what_jdbgen_encrypted() {
    let vectors = vectors();
    let decryptor = Decryptor::new(&vectors.master);
    for vector in &vectors.values {
        let opened = decryptor
            .decrypt(&vector.legacy, "userPassword")
            .unwrap_or_else(|error| panic!("{:?}: {error}", vector.plain));
        assert_eq!(opened, vector.plain);
    }
}

#[test]
fn the_vectors_cover_the_values_a_configuration_actually_holds() {
    let vectors = vectors();
    let plain: Vec<&str> = vectors.values.iter().map(|v| v.plain.as_str()).collect();
    assert!(plain.iter().any(|value| value.starts_with("jdbc:")));
    assert!(
        plain.contains(&""),
        "a connection with no password stores \"\""
    );
    assert!(
        plain.iter().any(|value| !value.is_ascii()),
        "jdbgen is used in Korean and a password is UTF-8"
    );
    assert!(
        vectors.values.iter().any(|v| v.plain.len() > 60),
        "a value longer than one AES block"
    );
}

#[test]
fn a_wrong_master_password_fails_the_gcm_tag() {
    let vectors = vectors();
    let decryptor = Decryptor::new(&vectors.wrong_master);
    let vector = &vectors.values[0];
    assert!(matches!(
        decryptor.decrypt(&vector.v2, "userPassword"),
        Err(Error::WrongPassword)
    ));
}

#[test]
fn a_wrong_master_password_fails_the_legacy_padding_the_same_way() {
    let vectors = vectors();
    let decryptor = Decryptor::new(&vectors.wrong_master);
    // Every legacy vector, because the superseded scheme has no authentication
    // tag: the padding is the only evidence, and one value in a hundred pads
    // plausibly under the wrong key. All of them failing is the assertion worth
    // making.
    for vector in vectors.values.iter().filter(|v| !v.legacy.is_empty()) {
        assert!(
            matches!(
                decryptor.decrypt(&vector.legacy, "userPassword"),
                Err(Error::WrongPassword)
            ),
            "{:?} opened under the wrong password",
            vector.plain
        );
    }
}

#[test]
fn a_tampered_ciphertext_is_a_wrong_password_and_not_a_plaintext() {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let vectors = vectors();
    let decryptor = Decryptor::new(&vectors.master);
    let vector = &vectors.values[0];
    let mut raw = BASE64
        .decode(vector.v2.strip_prefix("ENC2:").unwrap())
        .unwrap();
    // One bit of the ciphertext body, past the salt and the nonce.
    let last = raw.len() - 1;
    raw[last] ^= 0x01;
    let tampered = format!("ENC2:{}", BASE64.encode(&raw));
    assert!(matches!(
        decryptor.decrypt(&tampered, "connectionUrl"),
        Err(Error::WrongPassword)
    ));
}

#[test]
fn a_tampered_salt_is_a_wrong_key_rather_than_a_different_value() {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    let vectors = vectors();
    let decryptor = Decryptor::new(&vectors.master);
    let mut raw = BASE64
        .decode(vectors.values[0].v2.strip_prefix("ENC2:").unwrap())
        .unwrap();
    raw[0] ^= 0xff;
    let tampered = format!("ENC2:{}", BASE64.encode(&raw));
    assert!(matches!(
        decryptor.decrypt(&tampered, "connectionUrl"),
        Err(Error::WrongPassword)
    ));
}
