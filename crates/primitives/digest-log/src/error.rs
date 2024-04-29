/// Error that may occur while searching a Deoxys \[Log\] in the \[Digest\]
///
/// As for now only one single Deoxys \[Log\] is expected per \[Digest\].
/// No more, no less.
#[derive(Clone, Debug)]
pub enum FindLogError {
    /// There was no Deoxys \[Log\] in the \[Digest\]
    NotLog,
    /// There was multiple Deoxys \[Log\] in the \[Digest\]
    MultipleLogs,
}

#[cfg(feature = "std")]
impl std::error::Error for FindLogError {}

impl core::fmt::Display for FindLogError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            FindLogError::NotLog => write!(f, "Deoxys log not found"),
            FindLogError::MultipleLogs => write!(f, "Multiple Deoxys logs found"),
        }
    }
}
