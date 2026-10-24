//! The Claude Code version the plugin says it is, and the floor Anthropic raises it to.

use std::fmt;
use std::str::FromStr;

/// What the plugin claims to be, until Anthropic asks for newer or the config says otherwise.
/// The number is the current Claude Code release (`npm view @anthropic-ai/claude-code version`).
pub(crate) const DEFAULT: Version = Version::new(2, 1, 310);

/// The phrase the API puts after the version it requires of a Claude Code that is too old.
const REQUIRED_SUFFIX: &str = " or newer is required";

/// A bare `major.minor.patch` version, compared numerically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version {
    major: u32,
    minor: u32,
    patch: u32,
}

impl Version {
    pub(crate) const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl FromStr for Version {
    type Err = ();

    fn from_str(text: &str) -> Result<Self, ()> {
        let mut parts = text
            .split('.')
            .map(|part| part.parse::<u32>().map_err(|_| ()));
        let version = Self::new(
            parts.next().ok_or(())??,
            parts.next().ok_or(())??,
            parts.next().ok_or(())??,
        );
        parts.next().is_none().then_some(version).ok_or(())
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The version an error message says a request has to claim at least, when it says so:
/// "Claude Code 2.1.260 does not support this model; version 2.1.280 or newer is required."
pub(crate) fn required_by(message: &str) -> Option<Version> {
    let (before, _) = message.split_once(REQUIRED_SUFFIX)?;
    before.rsplit(' ').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_not_by_text() {
        let older: Version = "2.1.75".parse().unwrap();
        let newer: Version = "2.1.280".parse().unwrap();

        assert!(newer > older);
        assert_eq!(newer.to_string(), "2.1.280");
    }

    #[test]
    fn only_a_bare_three_part_version_parses() {
        for bad in ["", "2.1", "2.1.0.4", "v2.1.0", "2.1.x", "2.1.-3", "latest"] {
            assert!(bad.parse::<Version>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_floor_is_the_version_before_the_phrase_not_the_one_that_was_rejected() {
        let message = "HTTP 400: Claude Code 2.1.260 does not support this model; version \
                       2.1.280 or newer is required. Please update.";

        assert_eq!(required_by(message), Some(Version::new(2, 1, 280)));
    }

    #[test]
    fn a_message_that_names_no_floor_gives_none() {
        assert_eq!(required_by("HTTP 400: prompt is too long"), None);
        assert_eq!(required_by("version later or newer is required"), None);
    }
}
