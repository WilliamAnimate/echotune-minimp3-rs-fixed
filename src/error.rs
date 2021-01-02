use thiserror::Error;

/// Errors encountered by the MP3 decoder.
#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0}")]
    /// An error caused by some IO operation required during decoding.
    Io(#[from] std::io::Error),
    #[error("Insufficient data")]
    /// The decoder tried to parse a frame from its internal buffer, but there
    /// was not enough.
    InsufficientData,
    #[error("Skipped data")]
    /// The decoder encountered data which was not a frame (ie, ID3 data), and
    /// skipped it.
    SkippedData,
    #[error("End of reader")]
    /// The decoder has reached the end of the provided reader.
    Eof,
    /// Minimp3 had a memory error, likely allocation
    MiniMemory,
    /// Minimp3 had an IO error
    MiniIo,
    /// Minimp3 had a parameter error
    MiniParam,
    /// Minimp3 had an user error
    MiniUser,
    /// Minimp3 had an decoder error, likely the sampling rate/channels/layer changed mid stream
    MiniDecode,
    /// Minimp3 returned an unknown error code
    MiniUnknown,
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::Io(err)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        match self {
            Error::Io(io_err) => write!(f, "IO error: {}", io_err),
            _ => f.write_str(self.description()),
        }
    }
}

impl StdError for Error {
    fn description(&self) -> &str {
        use Error::*;
        match self {
            Io(io_err) => io_err.description(),
            InsufficientData => "Insufficient data",
            SkippedData => "Skipped data",
            Eof => "End of reader",
            MiniMemory => "Minimp3 memory error",
            MiniIo => "Minimp3 io error",
            MiniParam => "Minimp3 parameter error",
            MiniUser => "Minimp3 user error",
            MiniDecode => "Minimp3 decoder error",
            MiniUnknown => "Unknown error",
        }
    }

    fn cause(&self) -> Option<&dyn StdError> {
        match self {
            Error::Io(io_err) => Some(io_err),
            _ => None,
        }
    }
}

pub(crate) fn from_mini_error(ec: i32) -> Result<(), Error> {
    match ec {
        0 => Ok(()),
        ffi::MP3D_E_MEMORY => Err(Error::MiniMemory),
        ffi::MP3D_E_IOERROR => Err(Error::MiniIo),
        ffi::MP3D_E_PARAM => Err(Error::MiniParam),
        ffi::MP3D_E_USER => Err(Error::MiniUser),
        ffi::MP3D_E_DECODE => Err(Error::MiniDecode),
        _ => Err(Error::MiniUnknown),
    }
}
