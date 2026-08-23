//! What can go wrong between a `config.json` on disk and a [`Decrypted`]
//! configuration in memory.
//!
//! [`Decrypted`]: crate::Decrypted

use std::path::PathBuf;

use thiserror::Error;

/// A jdbgen configuration could not be read or could not be decrypted.
///
/// The variants are what the import wizard has to tell apart: a file that is
/// not there or is not JSON is a different conversation from a master password
/// that does not open it, and only the second one is worth retrying.
#[derive(Debug, Error)]
pub enum Error {
    /// The file could not be read.
    #[error("cannot read '{path}': {source}")]
    Read {
        /// Path that was being read.
        path: PathBuf,
        /// The underlying I/O failure.
        source: std::io::Error,
    },

    /// The file was read but is not a jdbgen configuration.
    #[error("'{path}' is not a jdbgen configuration: {source}")]
    Parse {
        /// Path that was being parsed.
        path: PathBuf,
        /// Where the JSON stopped making sense.
        source: serde_json::Error,
    },

    /// The master password does not open this configuration.
    ///
    /// This is the answer to every *cryptographic* failure, whichever scheme
    /// the value was written in: a GCM tag that does not verify, a PKCS#5
    /// padding that is not a padding, or a plaintext that is not UTF-8. They
    /// are one and the same event — the key was wrong — and telling them apart
    /// would only describe the shape of the guess back to whoever made it.
    #[error("the master password does not open this configuration")]
    WrongPassword,

    /// An encrypted value is damaged rather than merely unreadable.
    ///
    /// Kept apart from [`Error::WrongPassword`] because no password can cause
    /// it: the envelope itself is not Base64, or is shorter than the salt, the
    /// initialisation vector and the authentication tag it has to carry. The
    /// wizard says *this file is damaged* rather than *try again*.
    #[error("the value of '{field}' is damaged: {reason}")]
    Malformed {
        /// jdbgen's name for the field, e.g. `userPassword`.
        field: &'static str,
        /// What was wrong with the envelope.
        reason: &'static str,
    },
}

/// Result of every fallible operation in this crate.
pub type Result<T> = std::result::Result<T, Error>;
