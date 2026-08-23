//! jdbgen's two encryption schemes, read-only.
//!
//! rudbgen encrypts nothing (architecture document, D5): this module exists so
//! that a configuration written by jdbgen can be opened once, its three
//! encrypted fields moved into the OS keychain, and the master password
//! forgotten. It is a transcription of jdbgen's `comart.utils.StrUtils`, and
//! the constants below are that file's constants.
//!
//! **Current form**, marked by an `ENC2:` prefix:
//!
//! ```text
//! "ENC2:" + Base64( salt(16) || iv(12) || AES-256-GCM(ciphertext || tag) )
//! ```
//!
//! with the key stretched from the master password and the salt *carried in the
//! value* by PBKDF2-HMAC-SHA256 over 210,000 iterations. Because the salt
//! travels with the value, a file written across several sessions holds several
//! salts and stays readable.
//!
//! **Superseded form**, with no prefix: plain Base64 of an AES-128/CBC/PKCS#5
//! ciphertext whose key is the first half of SHA-256(master) and whose
//! initialisation vector is the second. It has no authentication tag, so a
//! wrong password is detected only by the padding — which is why
//! [`Decryptor::decrypt`] answers [`Error::WrongPassword`] to a padding failure
//! and to a plaintext that is not UTF-8 alike.
//!
//! The key derivation is deliberately expensive and a configuration decrypts
//! three values per connection, so [`Decryptor`] caches the derived key per
//! salt exactly as jdbgen's `KEY_CACHE` does. That is what makes decrypting a
//! file of twenty connections one derivation rather than sixty.

use std::cell::RefCell;
use std::collections::HashMap;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, KeyIvInit};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use crate::error::{Error, Result};

/// Marks a value written in the current format.
const ENC_V2_PREFIX: &str = "ENC2:";
/// Iteration count of the key derivation, jdbgen's `PBKDF2_ITERATIONS`.
const PBKDF2_ITERATIONS: u32 = 210_000;
/// Length of the derived key, in bytes (jdbgen's `KEY_BITS` / 8).
const KEY_LEN: usize = 32;
/// Length of the salt carried by a v2 value, in bytes.
const SALT_LEN: usize = 16;
/// Length of the GCM initialisation vector, in bytes.
const GCM_IV_LEN: usize = 12;
/// Length of the GCM authentication tag, in bytes.
const GCM_TAG_LEN: usize = 16;

/// Opens the encrypted fields of one jdbgen configuration.
///
/// Holds the master password for as long as the import runs and no longer: the
/// bytes are wiped when the decryptor is dropped, and nothing in this crate
/// writes them anywhere. The password rudbgen keeps afterwards is the *database*
/// password, and it goes to the OS keychain through
/// [`rudbgen_core::SecretStore`] — never back into a file.
pub struct Decryptor {
    /// The master password, UTF-8, wiped on drop.
    master: Vec<u8>,
    /// Derived keys by salt, jdbgen's `KEY_CACHE`.
    ///
    /// A `RefCell` rather than a `&mut self` on the decrypt path: the cache is
    /// an optimisation and not a fact about the value being read, and a caller
    /// holding the decryptor immutably while walking a configuration is the
    /// natural shape.
    keys: RefCell<HashMap<[u8; SALT_LEN], [u8; KEY_LEN]>>,
    /// Key and initialisation vector of the superseded scheme, derived once.
    legacy: RefCell<Option<([u8; 16], [u8; 16])>>,
}

impl Decryptor {
    /// Prepare to read values encrypted under `master`.
    ///
    /// Nothing is derived here: a configuration whose values are all in the
    /// legacy format never runs PBKDF2 at all.
    pub fn new(master: &str) -> Self {
        Self {
            master: master.as_bytes().to_vec(),
            keys: RefCell::new(HashMap::new()),
            legacy: RefCell::new(None),
        }
    }

    /// Decrypt one stored value, in whichever of the two forms it was written.
    ///
    /// `field` names the field for [`Error::Malformed`]; it is never shown to a
    /// user without translation. An empty or blank value decrypts to an empty
    /// string, which is jdbgen's own answer — the three encrypted fields are
    /// optional and a connection with no password stores `""`.
    ///
    /// # Errors
    ///
    /// [`Error::WrongPassword`] when the key does not open the value, and
    /// [`Error::Malformed`] when the envelope is not one at all.
    pub fn decrypt(&self, value: &str, field: &'static str) -> Result<String> {
        if value.trim().is_empty() {
            return Ok(String::new());
        }
        match value.strip_prefix(ENC_V2_PREFIX) {
            Some(payload) => self.decrypt_v2(payload, field),
            None => self.decrypt_legacy(value, field),
        }
    }

    /// Whether a value in the superseded format has been read.
    ///
    /// jdbgen raises the same flag to rewrite the file in the current format.
    /// Here it is only reported: the import writes rudbgen's stores and leaves
    /// the jdbgen configuration exactly as it found it.
    pub fn is_legacy(value: &str) -> bool {
        !value.trim().is_empty() && !value.starts_with(ENC_V2_PREFIX)
    }

    /// The current format: salt and initialisation vector travel with the
    /// value, and the tag says whether the key was right.
    fn decrypt_v2(&self, payload: &str, field: &'static str) -> Result<String> {
        let raw = BASE64
            .decode(payload.trim())
            .map_err(|_| Error::Malformed {
                field,
                reason: "the payload after 'ENC2:' is not Base64",
            })?;
        if raw.len() < SALT_LEN + GCM_IV_LEN + GCM_TAG_LEN {
            return Err(Error::Malformed {
                field,
                reason: "the value is shorter than its salt, nonce and tag",
            });
        }
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&raw[..SALT_LEN]);
        let iv = &raw[SALT_LEN..SALT_LEN + GCM_IV_LEN];
        let body = &raw[SALT_LEN + GCM_IV_LEN..];

        let key = self.derive(&salt);
        let cipher = Aes256Gcm::new_from_slice(&key).expect("a 32 byte AES-256 key");
        let nonce = Nonce::try_from(iv).expect("a 12 byte GCM nonce");
        let plain = cipher
            .decrypt(&nonce, body)
            .map_err(|_| Error::WrongPassword)?;
        String::from_utf8(plain).map_err(|_| Error::WrongPassword)
    }

    /// The superseded format: no salt, no tag, and the padding is the only
    /// evidence that the key was right.
    fn decrypt_legacy(&self, value: &str, field: &'static str) -> Result<String> {
        let raw = BASE64.decode(value.trim()).map_err(|_| Error::Malformed {
            field,
            reason: "the value is neither 'ENC2:'-prefixed nor Base64",
        })?;
        if raw.is_empty() || raw.len() % 16 != 0 {
            return Err(Error::Malformed {
                field,
                reason: "the value is not a whole number of AES blocks",
            });
        }
        let (key, iv) = self.legacy_key();
        let plain = cbc::Decryptor::<aes::Aes128>::new(&key.into(), &iv.into())
            .decrypt_padded_vec::<Pkcs7>(&raw)
            .map_err(|_| Error::WrongPassword)?;
        String::from_utf8(plain).map_err(|_| Error::WrongPassword)
    }

    /// Stretch the master password with `salt`, remembering the result.
    fn derive(&self, salt: &[u8; SALT_LEN]) -> [u8; KEY_LEN] {
        if let Some(key) = self.keys.borrow().get(salt) {
            return *key;
        }
        let mut key = [0u8; KEY_LEN];
        pbkdf2::pbkdf2_hmac::<Sha256>(&self.master, salt, PBKDF2_ITERATIONS, &mut key);
        self.keys.borrow_mut().insert(*salt, key);
        key
    }

    /// Key and initialisation vector of the superseded scheme: the two halves
    /// of SHA-256(master).
    fn legacy_key(&self) -> ([u8; 16], [u8; 16]) {
        if let Some(pair) = *self.legacy.borrow() {
            return pair;
        }
        let digest = Sha256::digest(&self.master);
        let mut key = [0u8; 16];
        let mut iv = [0u8; 16];
        key.copy_from_slice(&digest[..16]);
        iv.copy_from_slice(&digest[16..32]);
        *self.legacy.borrow_mut() = Some((key, iv));
        (key, iv)
    }
}

impl Drop for Decryptor {
    /// Wipe the master password and every key derived from it.
    ///
    /// Best effort, and the whole of what makes this crate's promise — the
    /// master password is asked for once and never stored — true of memory as
    /// well as of disk.
    fn drop(&mut self) {
        self.master.zeroize();
        for key in self.keys.borrow_mut().values_mut() {
            key.zeroize();
        }
        if let Some((key, iv)) = self.legacy.borrow_mut().as_mut() {
            key.zeroize();
            iv.zeroize();
        }
    }
}

impl std::fmt::Debug for Decryptor {
    /// Never renders the master password, nor how long it is.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decryptor")
            .field("master", &"<hidden>")
            .field("cached_keys", &self.keys.borrow().len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_value_decrypts_to_an_empty_string_without_a_password() {
        let dec = Decryptor::new("whatever");
        assert_eq!(dec.decrypt("", "userPassword").unwrap(), "");
        assert_eq!(dec.decrypt("   ", "userPassword").unwrap(), "");
    }

    #[test]
    fn a_truncated_v2_envelope_is_damage_rather_than_a_wrong_password() {
        let dec = Decryptor::new("whatever");
        let short = format!("ENC2:{}", BASE64.encode([0u8; 8]));
        assert!(matches!(
            dec.decrypt(&short, "userName"),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn a_v2_payload_that_is_not_base64_is_damage() {
        let dec = Decryptor::new("whatever");
        assert!(matches!(
            dec.decrypt("ENC2:!!!not base64!!!", "userName"),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn a_legacy_value_that_is_not_a_whole_number_of_blocks_is_damage() {
        let dec = Decryptor::new("whatever");
        assert!(matches!(
            dec.decrypt(&BASE64.encode([0u8; 7]), "connectionUrl"),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn the_legacy_form_is_the_one_without_the_prefix() {
        assert!(Decryptor::is_legacy("abcd"));
        assert!(!Decryptor::is_legacy("ENC2:abcd"));
        assert!(!Decryptor::is_legacy(""));
    }

    #[test]
    fn one_salt_is_stretched_once_however_many_values_share_it() {
        let dec = Decryptor::new("correct horse battery staple");
        let salt = [7u8; SALT_LEN];
        let first = dec.derive(&salt);
        let second = dec.derive(&salt);
        assert_eq!(first, second);
        assert_eq!(dec.keys.borrow().len(), 1);
    }

    #[test]
    fn the_debug_rendering_never_carries_the_master_password() {
        let dec = Decryptor::new("correct horse battery staple");
        let rendered = format!("{dec:?}");
        assert!(!rendered.contains("horse"), "{rendered}");
    }
}
