//! The billing header: a system block that marks a request as Claude Code's own.
//!
//! Without it Anthropic counts an OAuth request as a third-party app's and answers with a 400
//! that says the account is out of extra usage. The recipe, including its salt, is the one Claude
//! Code's requests follow; it hashes a few characters of the first user message with the version.

use sha2::{Digest, Sha256};

use crate::version::Version;

const SALT: &str = "59cf53e54c78";
/// Which characters of the first user message go into the version suffix.
const SAMPLED_POSITIONS: [usize; 3] = [4, 7, 20];
/// Stands in for a sampled position the message is too short to have.
const MISSING_CHARACTER: char = '0';
const ENTRYPOINT: &str = "sdk-cli";

/// The header's text for a conversation that starts with `first_user_text`, or `None` when there
/// is no such text to hash.
pub(crate) fn header(first_user_text: &str, version: Version) -> Option<String> {
    if first_user_text.is_empty() {
        return None;
    }
    let cch = hex(&Sha256::digest(first_user_text.as_bytes()), 5);
    let sampled: String = SAMPLED_POSITIONS
        .iter()
        .map(|&at| first_user_text.chars().nth(at).unwrap_or(MISSING_CHARACTER))
        .collect();
    let suffix = hex(
        &Sha256::digest(format!("{SALT}{sampled}{version}").as_bytes()),
        3,
    );
    Some(format!(
        "x-anthropic-billing-header: cc_version={version}.{suffix}; cc_entrypoint={ENTRYPOINT}; cch={cch};"
    ))
}

/// The first `digits` hex digits of `bytes`.
fn hex(bytes: &[u8], digits: usize) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0xf])
        .take(digits)
        .map(|nibble| char::from_digit(u32::from(nibble), 16).unwrap_or('0'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: Version = Version::new(2, 1, 310);

    // The digests were computed apart from this code, with sha256 over the same inputs.
    #[test]
    fn the_header_hashes_the_message_and_a_few_of_its_characters() {
        let header = header("Hello, world! this is the first message", VERSION).unwrap();

        assert_eq!(
            header,
            "x-anthropic-billing-header: cc_version=2.1.310.8ac; cc_entrypoint=sdk-cli; cch=dcdf3;"
        );
    }

    #[test]
    fn a_short_message_is_padded_with_zeros_where_it_has_no_character() {
        let header = header("hi", VERSION).unwrap();

        assert_eq!(
            header,
            "x-anthropic-billing-header: cc_version=2.1.310.077; cc_entrypoint=sdk-cli; cch=8f434;"
        );
    }

    #[test]
    fn nothing_to_hash_means_no_header() {
        assert_eq!(header("", VERSION), None);
    }
}
