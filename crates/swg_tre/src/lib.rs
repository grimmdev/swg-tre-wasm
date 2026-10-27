//! This library handles reading from and creating TRE files used by Star Wars Galaxies
//!
//! A TRE file is an archive file that allows compression of it's components. It is written with the Little Endian
//! byte order and is structured as follows:
//!
//! * 4 Byte Magic Marker - "TREE"
//! * 4 Byte Version - "0005"
//! * 36 Byte Header - [`crate::types::TreHeader`]
//! * Data for each individually compressable record
//! * A list of record metadata ([`crate::types::TreRecord`]), where the whole block can be compressed
//! * A list of null terminated name strings, where the whole block can be compressed
//!

pub mod compression;
pub mod error;
pub mod read;
pub mod types;
pub mod write;

pub use compression::CompressionMethod;
pub use read::TreArchive;
pub use write::TreWriter;
