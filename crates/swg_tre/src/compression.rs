//! Block compression and decompression handling.

use std::io::{self, Read, Seek, Write};

use binrw::{io::NoSeek, BinRead, BinWrite};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use tracing::instrument;

use crate::error::Result;

/// Identifies the storage format used to compress a block inside the TRE file
///
/// When creating TRE files, you may choose the method to use for the records and names via [`crate::write::TreWriterOptions`].
///
/// Files added to the TRE can specify it's compression method via [`crate::write::TreWriter::start_file`]
///
#[derive(BinRead, BinWrite, Debug, Copy, Clone, Default, PartialEq)]
#[brw(repr=u32)]
pub enum CompressionMethod {
    /// Stores the data as it is
    None = 0,

    /// Compress the data using Zlib
    #[default]
    Zlib = 2,
}

impl From<u32> for CompressionMethod {
    fn from(value: u32) -> Self {
        match value {
            0 => CompressionMethod::None,
            2 => CompressionMethod::Zlib,
            _ => unreachable!(),
        }
    }
}

pub(crate) enum TreBlockReader<'a, W: Read + Seek> {
    Raw(io::Take<&'a mut W>),
    Compressed(Box<ZlibDecoder<io::Take<&'a mut W>>>),
}

impl<'a, W: Read + Seek> TreBlockReader<'a, W> {
    #[tracing::instrument(skip(reader))]
    pub fn new(
        reader: &'a mut W,
        start: u64,
        limit: u64,
        compression: CompressionMethod,
    ) -> Result<Self> {
        reader.seek(io::SeekFrom::Start(start))?;

        let limit_reader = reader.by_ref().take(limit);
        Ok(match compression {
            CompressionMethod::None => TreBlockReader::Raw(limit_reader),
            CompressionMethod::Zlib => {
                TreBlockReader::Compressed(Box::new(ZlibDecoder::new(limit_reader)))
            }
        })
    }

    #[instrument(skip(self), err)]
    pub fn into_inner(self) -> io::Result<io::Take<&'a mut W>> {
        match self {
            TreBlockReader::Raw(r) => Ok(r),
            TreBlockReader::Compressed(r) => Ok(r.into_inner()),
        }
    }
}

impl<W: Read + Seek> Seek for TreBlockReader<'_, W> {
    #[instrument(skip(self), err)]
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        match self {
            TreBlockReader::Raw(r) => NoSeek::new(r).seek(pos),
            TreBlockReader::Compressed(r) => NoSeek::new(r).seek(pos),
        }
    }
}

impl<W: Read + Seek> Read for TreBlockReader<'_, W> {
    #[instrument(skip(self), err)]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            TreBlockReader::Raw(r) => r.read(buf),
            TreBlockReader::Compressed(r) => r.read(buf),
        }
    }

    #[instrument(skip(self), err)]
    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        match self {
            TreBlockReader::Raw(r) => r.read_exact(buf),
            TreBlockReader::Compressed(r) => r.read_exact(buf),
        }
    }

    #[instrument(skip(self), err)]
    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        match self {
            TreBlockReader::Raw(r) => r.read_to_end(buf),
            TreBlockReader::Compressed(r) => r.read_to_end(buf),
        }
    }

    #[instrument(skip(self), err)]
    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        match self {
            TreBlockReader::Raw(r) => r.read_to_string(buf),
            TreBlockReader::Compressed(r) => r.read_to_string(buf),
        }
    }
}

pub(crate) enum TreBlockWriter<W: Write + Seek> {
    Raw(W, usize),
    Compressed(Box<ZlibEncoder<W>>),
}

impl<W: Write + Seek> TreBlockWriter<W> {
    #[tracing::instrument(skip(writer))]
    pub fn new(writer: W, compression: CompressionMethod) -> Self {
        match compression {
            CompressionMethod::None => TreBlockWriter::Raw(writer, 0),
            CompressionMethod::Zlib => TreBlockWriter::Compressed(Box::new(ZlibEncoder::new(
                writer,
                Compression::default(),
            ))),
        }
    }

    #[instrument(skip(self), err)]
    pub fn finalize(self) -> io::Result<W> {
        match self {
            TreBlockWriter::Raw(r, _) => Ok(r),
            TreBlockWriter::Compressed(r) => r.finish(),
        }
    }

    pub fn total_in(&self) -> u64 {
        match self {
            TreBlockWriter::Raw(_, c) => *c as u64,
            TreBlockWriter::Compressed(r) => r.total_in(),
        }
    }
}

impl<W: Write + Seek> Seek for TreBlockWriter<W> {
    #[instrument(skip(self), err)]
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        match self {
            TreBlockWriter::Raw(r, _) => NoSeek::new(r).seek(pos),
            TreBlockWriter::Compressed(r) => NoSeek::new(r).seek(pos),
        }
    }
}

impl<W: Write + Seek> Write for TreBlockWriter<W> {
    #[instrument(skip(self), err)]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            TreBlockWriter::Raw(r, c) => {
                let written = r.write(buf)?;
                *c += written;
                Ok(written)
            }
            TreBlockWriter::Compressed(r) => r.write(buf),
        }
    }

    #[instrument(skip(self), err)]
    fn flush(&mut self) -> io::Result<()> {
        match self {
            TreBlockWriter::Raw(r, _) => r.flush(),
            TreBlockWriter::Compressed(r) => r.flush(),
        }
    }
}
