//! Error types that can be emitted from this library

use miette::Diagnostic;
use thiserror::Error;

/// Error type for library
#[derive(Error, Diagnostic, Debug)]
pub enum Error {
    /// Transparent warpper for [`std::io::Error`]
    #[error(transparent)]
    IOError(#[from] std::io::Error),

    /// Transparent warpper for [`binrw::Error`]
    #[error(transparent)]
    BinRWError(#[from] binrw::Error),

    /// file is an invalid tre archive
    #[error("file is an invalid tre archive")]
    InvalidArchive,

    /// unable to find requested file
    #[error("unable to find requested file")]
    FileNotFound(#[from] FileNotFoundError),

    /// {0}
    #[error("{0}")]
    CustomError(String),
}

/// Error type to provide further information when a file has not been found
#[derive(Error, Diagnostic, Debug)]
#[error("unable to find requested file")]
pub enum FileNotFoundError {
    /// at index {0}
    #[error("at index {0}")]
    Index(usize),

    /// by name {0}
    #[error("by name {0}")]
    Name(String),
}

/// Generic result type with crate's Error as its error variant
pub type Result<T> = core::result::Result<T, Error>;
