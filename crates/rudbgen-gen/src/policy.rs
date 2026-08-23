//! What a run does when the file it is about to write is already there.

use std::fmt;
use std::path::Path;

use rudbgen_core::OverwritePolicy;

/// One answer to one conflict.
///
/// The two `*All` answers are what makes the *ask* policy usable on a run of
/// two hundred files: they answer the rest of the run in one click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Replace this file.
    Overwrite,
    /// Keep this file and count the pair as skipped.
    Skip,
    /// Replace this file and every later conflict, without asking again.
    OverwriteAll,
    /// Keep this file and every later conflict, without asking again.
    SkipAll,
    /// Stop the run here.
    Cancel,
}

/// The policy of one run: [`rudbgen_core::OverwritePolicy`] with the question
/// of the *ask* case attached.
///
/// The callback runs on the thread the job runs on. The application is
/// expected to send the question to the UI and block on the answer; anything
/// that blocks forever hangs the job, which is why [`Decision::Cancel`] is one
/// of the answers rather than a timeout here.
pub enum Overwrite {
    /// Replace whatever is there.
    Overwrite,
    /// Keep whatever is there, and count the pair as skipped.
    Skip,
    /// Ask, once per conflicting file, until an answer settles the rest.
    Ask(Box<dyn Fn(&Path) -> Decision + Send>),
}

impl Overwrite {
    /// The saved policy, with `ask` used for [`OverwritePolicy::Ask`].
    ///
    /// There is no `From<OverwritePolicy>`: the *ask* case cannot be built
    /// without somewhere to ask, and silently turning it into one of the other
    /// two would be destructive in one direction or the other.
    pub fn from_policy<F>(policy: OverwritePolicy, ask: F) -> Self
    where
        F: Fn(&Path) -> Decision + Send + 'static,
    {
        match policy {
            OverwritePolicy::Overwrite => Overwrite::Overwrite,
            OverwritePolicy::Skip => Overwrite::Skip,
            OverwritePolicy::Ask => Overwrite::Ask(Box::new(ask)),
        }
    }
}

impl fmt::Debug for Overwrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Overwrite::Overwrite => f.write_str("Overwrite"),
            Overwrite::Skip => f.write_str("Skip"),
            Overwrite::Ask(_) => f.write_str("Ask(<callback>)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_policy_carries_over() {
        let ask = |_: &Path| Decision::Skip;
        assert!(matches!(
            Overwrite::from_policy(OverwritePolicy::Overwrite, ask),
            Overwrite::Overwrite
        ));
        assert!(matches!(
            Overwrite::from_policy(OverwritePolicy::Skip, ask),
            Overwrite::Skip
        ));
        assert!(matches!(
            Overwrite::from_policy(OverwritePolicy::Ask, ask),
            Overwrite::Ask(_)
        ));
    }

    #[test]
    fn the_callback_stays_out_of_the_debug_output() {
        let printed = format!(
            "{:?}",
            Overwrite::from_policy(OverwritePolicy::Ask, |_| { Decision::Cancel })
        );
        assert_eq!(printed, "Ask(<callback>)");
    }
}
