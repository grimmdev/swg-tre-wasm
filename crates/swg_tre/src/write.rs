//! Types for writing TRE archives
//!

use binrw::BinWrite;
use bon::{builder, Builder};
use byteorder::WriteBytesExt;
use md5::{Digest, Md5};
use std::fmt::Debug;
use std::io::{self, Cursor, Seek, Write};
use std::mem;
use tracing::{instrument, Level};

use super::compression::CompressionMethod;
use crate::compression::TreBlockWriter;
use crate::error::Result;
use crate::types::{TreHeader, TreRecord};

/// Options for how the TRE file should be written
#[derive(Debug, Clone, Copy, Builder)]
pub struct TreWriterOptions {
    /// The compression method to use for the record block
    #[builder(default)]
    pub record_compression: CompressionMethod,

    /// The compression method to use for the name block
    #[builder(default)]
    pub name_compression: CompressionMethod,
}

#[derive(Debug, Clone, Default)]
struct TreStats {
    info_offset: u32,
    name_offset: u32,
}

/// TRE archive generator
///
/// ```
/// # fn doit() -> swg_tre::error::Result<()>
/// # {
/// # use swg_tre::TreWriter;
/// use std::io::Write;
/// use swg_tre::write::TreWriterOptions;
///
/// // We use a buffer here, though you'd normally use a `File`
/// let mut buf = [0; 65536];
/// let mut tre = TreWriter::new(std::io::Cursor::new(&mut buf[..]), TreWriterOptions::builder()
///            .name_compression(swg_tre::CompressionMethod::None)
///            .record_compression(swg_tre::CompressionMethod::None)
///            .build());
///
/// tre.start_file("hello_world.txt", swg_tre::CompressionMethod::None)?;
/// tre.write(b"Hello, World!")?;
///
/// // Apply the changes you've made.
/// tre.finish()?;
///
/// # Ok(())
/// # }
/// # doit().unwrap();
/// ```
pub struct TreWriter<W: Write + Seek> {
    inner: W,
    writing_to_file: bool,
    info_block: TreBlockWriter<Cursor<Vec<u8>>>,
    data_block: TreBlockWriter<Cursor<Vec<u8>>>,
    name_block: TreBlockWriter<Cursor<Vec<u8>>>,
    hash_block: TreBlockWriter<Cursor<Vec<u8>>>,
    current_data_block: Option<TreBlockWriter<Cursor<Vec<u8>>>>,
    stats: TreStats,
    header: TreHeader,
    record: TreRecord,
}

impl<W: Write + Seek> TreWriter<W> {
    /// Initializes the archive.
    ///
    /// Before writing to this object, the [`TreWriter::start_file`] function should be called.
    /// After a successful write, the file remains open for writing. After a failed write, call
    /// [`TreWriter::is_writing_file`] to determine if the file remains open.
    pub fn new(inner: W, options: TreWriterOptions) -> TreWriter<W> {
        TreWriter {
            inner,
            writing_to_file: false,
            info_block: TreBlockWriter::new(Cursor::new(Vec::new()), options.record_compression),
            data_block: TreBlockWriter::new(Cursor::new(Vec::new()), CompressionMethod::None),
            current_data_block: None,
            name_block: TreBlockWriter::new(Cursor::new(Vec::new()), options.name_compression),
            hash_block: TreBlockWriter::new(Cursor::new(Vec::new()), CompressionMethod::None),
            stats: TreStats::default(),
            header: TreHeader {
                record_compression: options.record_compression,
                name_compression: options.name_compression,
                ..Default::default()
            },
            record: TreRecord::default(),
        }
    }

    /// Returns true if a file is currently open for writing.
    pub const fn is_writing_file(&self) -> bool {
        self.writing_to_file
    }

    /// Start a new file for with the requested compression.
    #[instrument(skip(self, name), err)]
    pub fn start_file(
        &mut self,
        name: impl ToString,
        compression: CompressionMethod,
    ) -> Result<()> {
        if self.writing_to_file {
            self.finish_file()?;
        }

        assert!(self.current_data_block.is_none());

        let _ = mem::replace(
            &mut self.current_data_block,
            Some(TreBlockWriter::new(Cursor::new(Vec::new()), compression)),
        );

        self.header.records += 1;
        {
            self.name_block.write_all(name.to_string().as_bytes())?;
            self.name_block.write_u8(0u8)?;
        }

        // Update Record
        self.record.data_compression = compression;
        self.record.checksum =
            crc::Crc::<u32>::new(&crc::CRC_32_BZIP2).checksum(name.to_string().as_bytes());

        self.record.data_offset = 36 + self.data_block.total_in() as u32;
        self.record.name_offset = self.stats.name_offset;

        self.stats.name_offset = self.name_block.total_in() as u32;

        self.writing_to_file = true;

        Ok(())
    }

    #[instrument(skip(self), err)]
    fn finish_file(&mut self) -> Result<()> {
        self.stats.info_offset += 24;

        let current_block = self
            .current_data_block
            .take()
            .expect("current data block should always be valid when finishing a file");

        let block_total_in = current_block.total_in();
        let current_block_data = current_block.finalize()?.into_inner();

        self.record.data_uncompressed = block_total_in as u32;
        self.record.data_compressed = current_block_data.len() as u32;

        self.record.write(&mut self.info_block)?;

        self.data_block.write_all(&current_block_data)?;

        let mut hasher = Md5::new();
        hasher.update(current_block_data);

        self.hash_block.write_all(&hasher.finalize())?;
        self.writing_to_file = false;

        Ok(())
    }

    /// Finish the last file and write all other TRE file structures
    ///
    /// This will return the writer, but one should normally not append any data to the end of the file.
    #[instrument(skip(self), err)]
    pub fn finish(mut self) -> Result<W> {
        if self.writing_to_file {
            self.finish_file()?;
        }

        let data_block = self.data_block.finalize()?.into_inner();
        self.header.record_start = 36 + data_block.len() as u32;

        let info_block = self.info_block.finalize()?.into_inner();
        self.header.record_compressed = info_block.len() as u32;

        self.header.name_uncompressed = self.name_block.total_in() as u32;
        let name_block = self.name_block.finalize()?.into_inner();
        self.header.name_compressed = name_block.len() as u32;

        self.header.write(&mut self.inner)?;
        self.inner.write_all(&data_block)?;
        self.inner.write_all(&info_block)?;
        self.inner.write_all(&name_block)?;
        self.inner
            .write_all(&self.hash_block.finalize()?.into_inner())?;

        Ok(self.inner)
    }
}

impl<W: Write + Seek> Write for TreWriter<W> {
    #[instrument(skip_all, err, ret(level = Level::TRACE), fields(size=buf.len()) )]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.writing_to_file {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "No file has been started",
            ));
        }
        self.current_data_block
            .as_mut()
            .expect("current data block should be initialized by the time we write")
            .write(buf)
    }

    #[instrument(skip(self), err)]
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_str_eq;
    use tracing_test::traced_test;

    use crate::error::Result;
    use crate::{
        compression::CompressionMethod,
        write::{TreWriter, TreWriterOptions},
    };
    use std::io::{Cursor, Write};

    #[traced_test]
    #[test]
    fn tre_uncompressed_empty_write() -> Result<()> {
        #[rustfmt::skip]
        let expected = vec![
            // Header
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
        ];

        let writer = TreWriter::new(
            Cursor::new(Vec::new()),
            TreWriterOptions::builder()
                .record_compression(CompressionMethod::None)
                .name_compression(CompressionMethod::None)
                .build(),
        );
        let result = writer.finish()?;
        assert_eq!(result.get_ref().len(), expected.len());
        assert_str_eq!(
            format!("{:02X?}", *result.get_ref()),
            format!("{:02X?}", expected)
        );

        Ok(())
    }

    #[traced_test]
    #[test]
    fn tre_compressed_empty_write() -> Result<()> {
        #[rustfmt::skip]
        let expected = vec![
            // Header
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00,
            0x08, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00,
            0x08, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            // Records
            0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01, 
            // Names
            0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01,
        ];

        let writer = TreWriter::new(
            Cursor::new(Vec::new()),
            TreWriterOptions::builder()
                .record_compression(CompressionMethod::Zlib)
                .name_compression(CompressionMethod::Zlib)
                .build(),
        );
        let result = writer.finish()?;
        assert_eq!(result.get_ref().len(), expected.len());
        assert_str_eq!(
            format!("{:02X?}", *result.get_ref()),
            format!("{:02X?}", expected)
        );

        Ok(())
    }

    #[traced_test]
    #[test]
    fn tre_uncompressed_without_data_write() -> Result<()> {
        #[rustfmt::skip]
        let expected = [
            // Header
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x01, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x18, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x0A, 0x00, 0x00, 0x00, 
            0x0A, 0x00, 0x00, 0x00,
            // Records
            0xAA, 0x30, 0x7E, 0x52, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            // Names
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x2E, 0x74, 0x78, 0x74, 0x00,
            // Hashes
            0xD4, 0x1D, 0x8C, 0xD9, 0x8F, 0x00, 0xB2, 0x04, 0xE9, 0x80, 0x09, 0x98, 0xEC, 0xF8, 0x42, 0x7E
        ];

        let mut writer = TreWriter::new(
            Cursor::new(Vec::new()),
            TreWriterOptions::builder()
                .record_compression(CompressionMethod::None)
                .name_compression(CompressionMethod::None)
                .build(),
        );
        writer.start_file("hello.txt", CompressionMethod::None)?;

        let result = writer.finish()?;
        assert_eq!(result.get_ref().len(), expected.len());
        assert_str_eq!(
            format!("{:02X?}", *result.get_ref()),
            format!("{:02X?}", expected)
        );

        Ok(())
    }

    #[traced_test]
    #[test]
    fn tre_uncompressed_multiple_entries_without_data_write() -> Result<()> {
        #[rustfmt::skip]
        let expected = [
            // Header
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x02, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x30, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x14, 0x00, 0x00, 0x00, 
            0x14, 0x00, 0x00, 0x00,
            // Records
            0xAA, 0x30, 0x7E, 0x52, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            
            0xDE, 0x6E, 0xB0, 0xD8, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x0A, 0x00, 0x00, 0x00,
            // Names
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x2E, 0x74, 0x78, 0x74, 0x00,
            0x77, 0x6F, 0x72, 0x6C, 0x64, 0x2E, 0x74, 0x78, 0x74, 0x00,
            // Hashes
            0xD4, 0x1D, 0x8C, 0xD9, 0x8F, 0x00, 0xB2, 0x04, 0xE9, 0x80, 0x09, 0x98, 0xEC, 0xF8, 0x42, 0x7E,
            0xD4, 0x1D, 0x8C, 0xD9, 0x8F, 0x00, 0xB2, 0x04, 0xE9, 0x80, 0x09, 0x98, 0xEC, 0xF8, 0x42, 0x7E
        ];

        let mut writer = TreWriter::new(
            Cursor::new(Vec::new()),
            TreWriterOptions::builder()
                .record_compression(CompressionMethod::None)
                .name_compression(CompressionMethod::None)
                .build(),
        );
        writer.start_file("hello.txt", CompressionMethod::None)?;
        writer.finish_file()?;

        writer.start_file("world.txt", CompressionMethod::None)?;

        let result = writer.finish()?;
        assert_eq!(result.get_ref().len(), expected.len());
        assert_str_eq!(
            format!("{:02X?}", *result.get_ref()),
            format!("{:02X?}", expected)
        );

        Ok(())
    }

    #[traced_test]
    #[test]
    fn tre_compressed_without_data_write() -> Result<()> {
        #[rustfmt::skip]
        let expected = [
            // Header
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x01, 0x00, 0x00, 0x00, 
            0x2C, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x18, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00,
            0x12, 0x00, 0x00, 0x00, 
            0x0A, 0x00, 0x00, 0x00,
            // Data
            0x78, 0x9C, 
            0x03, 0x00, 0x00, 0x00, 0x00, 0x01, 
            // Records
            0xAA, 0x30, 0x7E, 0x52, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00, 
            0x08, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            // Names
            0x78, 0x9C, 
            0xCB, 0x48, 0xCD, 0xC9, 0xC9, 0xD7, 0x2B, 0xA9, 0x28, 0x61, 0x00, 0x00, 
            0x15, 0x9B, 0x03, 0xA3,
            // Hashes
            0xFB, 0x0F, 0xC3, 0xAB, 0x8C, 0x05, 0x01, 0x79, 0xA3, 0x78, 0xDC, 0xB1, 0x03, 0x68, 0xAC, 0xF6
        ];

        let mut writer = TreWriter::new(
            Cursor::new(Vec::new()),
            TreWriterOptions::builder()
                .record_compression(CompressionMethod::None)
                .name_compression(CompressionMethod::Zlib)
                .build(),
        );
        writer.start_file("hello.txt", CompressionMethod::Zlib)?;

        let result = writer.finish()?;
        assert_eq!(result.get_ref().len(), expected.len());
        assert_str_eq!(
            format!("{:02X?}", *result.get_ref()),
            format!("{:02X?}", expected)
        );

        Ok(())
    }

    #[traced_test]
    #[test]
    fn tre_compressed_multiple_entries_without_data_write() -> Result<()> {
        #[rustfmt::skip]
        let expected = [
            // Header
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x02, 0x00, 0x00, 0x00, 
            0x34, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x30, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00,
            0x18, 0x00, 0x00, 0x00, 
            0x14, 0x00, 0x00, 0x00,
            // Data
            0x78, 0x9C, 
            0x03, 0x00, 0x00, 0x00, 0x00, 0x01, 
            
            0x78, 0x9C, 
            0x03, 0x00, 0x00, 0x00, 0x00, 0x01, 
            // Records
            0xAA, 0x30, 0x7E, 0x52, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00, 
            0x08, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            
            0xDE, 0x6E, 0xB0, 0xD8, 
            0x00, 0x00, 0x00, 0x00, 
            0x2C, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00, 
            0x08, 0x00, 0x00, 0x00, 
            0x0A, 0x00, 0x00, 0x00,
            // Names
            0x78, 0x9C, 
            0xCB, 0x48, 0xCD, 0xC9, 0xC9, 0xD7, 0x2B, 0xA9, 0x28, 0x61, 0x28, 0xCF, 0x2F, 0xCA, 0x49, 0x01, 0xB3, 0x00, 
            0x50, 0x68, 0x07, 0x59,
            // Hashes
            0xFB, 0x0F, 0xC3, 0xAB, 0x8C, 0x05, 0x01, 0x79, 0xA3, 0x78, 0xDC, 0xB1, 0x03, 0x68, 0xAC, 0xF6, 
            0xFB, 0x0F, 0xC3, 0xAB, 0x8C, 0x05, 0x01, 0x79, 0xA3, 0x78, 0xDC, 0xB1, 0x03, 0x68, 0xAC, 0xF6, 
        ];

        let mut writer = TreWriter::new(
            Cursor::new(Vec::new()),
            TreWriterOptions::builder()
                .record_compression(CompressionMethod::None)
                .name_compression(CompressionMethod::Zlib)
                .build(),
        );
        writer.start_file("hello.txt", CompressionMethod::Zlib)?;
        writer.finish_file()?;

        writer.start_file("world.txt", CompressionMethod::Zlib)?;

        let result = writer.finish()?;
        // assert_eq!(result.get_ref().len(), expected.len());
        assert_str_eq!(
            format!("{:02X?}", *result.get_ref()),
            format!("{:02X?}", expected)
        );

        Ok(())
    }

    #[test]
    fn tre_uncompressed_with_data_write() -> Result<()> {
        let file_data = [
            0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64,
        ];

        #[rustfmt::skip]
        let expected = [
            // Header
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x01, 0x00, 0x00, 0x00, 
            0x2F, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x18, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x0A, 0x00, 0x00, 0x00, 
            0x0A, 0x00, 0x00, 0x00,
            // Data
            0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64,
            // Records
            0xAA, 0x30, 0x7E, 0x52,
            0x0B, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x0B, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            // Names
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x2E, 0x74, 0x78, 0x74, 0x00,
            // Hashes
            0xB1, 0x0A, 0x8D, 0xB1, 0x64, 0xE0, 0x75, 0x41, 0x05, 0xB7, 0xA9, 0x9B, 0xE7, 0x2E, 0x3F, 0xE5
        ];

        let mut writer = TreWriter::new(
            Cursor::new(Vec::new()),
            TreWriterOptions::builder()
                .record_compression(CompressionMethod::None)
                .name_compression(CompressionMethod::None)
                .build(),
        );
        writer.start_file("hello.txt", CompressionMethod::None)?;
        writer.write_all(&file_data)?;

        let result = writer.finish()?;
        assert_eq!(result.get_ref().len(), expected.len());
        assert_str_eq!(
            format!("{:02X?}", *result.get_ref()),
            format!("{:02X?}", expected)
        );

        Ok(())
    }

    #[test]
    fn tre_uncompressed_multiple_entries_with_data_write() -> Result<()> {
        let hello_data = [
            0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64,
        ];

        let world_data = [
            0x57, 0x6F, 0x72, 0x6C, 0x64, 0x20, 0x48, 0x65, 0x6C, 0x6C, 0x6F,
        ];

        #[rustfmt::skip]
        let expected = [
            // Header
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x02, 0x00, 0x00, 0x00, 
            0x3A, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x30, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x14, 0x00, 0x00, 0x00, 
            0x14, 0x00, 0x00, 0x00,
            // Data
            0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64,
            0x57, 0x6F, 0x72, 0x6C, 0x64, 0x20, 0x48, 0x65, 0x6C, 0x6C, 0x6F,
            // Records
            0xAA, 0x30, 0x7E, 0x52,
            0x0B, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x0B, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,

            0xDE, 0x6E, 0xB0, 0xD8,
            0x0B, 0x00, 0x00, 0x00, 
            0x2F, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x0B, 0x00, 0x00, 0x00, 
            0x0A, 0x00, 0x00, 0x00,
            // Names
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x2E, 0x74, 0x78, 0x74, 0x00,
            0x77, 0x6F, 0x72, 0x6C, 0x64, 0x2E, 0x74, 0x78, 0x74, 0x00,
            // Hashes
            0xB1, 0x0A, 0x8D, 0xB1, 0x64, 0xE0, 0x75, 0x41, 0x05, 0xB7, 0xA9, 0x9B, 0xE7, 0x2E, 0x3F, 0xE5, 
            0x9F, 0xEF, 0x1D, 0xFD, 0x8F, 0xA4, 0x1F, 0x7A, 0xD0, 0x4D, 0x76, 0x0C, 0x77, 0xDE, 0xAB, 0x39
        ];

        let mut writer = TreWriter::new(
            Cursor::new(Vec::new()),
            TreWriterOptions::builder()
                .record_compression(CompressionMethod::None)
                .name_compression(CompressionMethod::None)
                .build(),
        );
        writer.start_file("hello.txt", CompressionMethod::None)?;
        writer.write_all(&hello_data)?;
        writer.finish_file()?;

        writer.start_file("world.txt", CompressionMethod::None)?;
        writer.write_all(&world_data)?;

        let result = writer.finish()?;
        assert_eq!(result.get_ref().len(), expected.len());
        assert_str_eq!(
            format!("{:02X?}", *result.get_ref()),
            format!("{:02X?}", expected)
        );

        Ok(())
    }

    #[test]
    fn tre_compressed_with_data_write() -> Result<()> {
        let file_data = [
            0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64,
        ];

        #[rustfmt::skip]
        let expected = [
            // Header
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x01, 0x00, 0x00, 0x00, 
            0x37, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x18, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x0A, 0x00, 0x00, 0x00, 
            0x0A, 0x00, 0x00, 0x00,
            // Data
            0x78, 0x9C, 
            0xF3, 0x48, 0xCD, 0xC9, 0xC9, 0x57, 0x08, 0xCF, 0x2F, 0xCA, 0x49, 0x01, 0x00, 0x18, 
            0x0B, 0x04, 0x1D,
            // Records
            0xAA, 0x30, 0x7E, 0x52,
            0x0B, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00, 
            0x13, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            // Names
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x2E, 0x74, 0x78, 0x74, 0x00,
            // Hashes
            0xED, 0xCF, 0xD4, 0xB0, 0xFA, 0x56, 0xDA, 0x3B, 0x9C, 0x65, 0x20, 0xFB, 0x8B, 0x79, 0x75, 0x61
        ];

        let mut writer = TreWriter::new(
            Cursor::new(Vec::new()),
            TreWriterOptions::builder()
                .record_compression(CompressionMethod::None)
                .name_compression(CompressionMethod::None)
                .build(),
        );
        writer.start_file("hello.txt", CompressionMethod::Zlib)?;
        writer.write_all(&file_data)?;

        let result = writer.finish()?;
        assert_eq!(result.get_ref().len(), expected.len());
        assert_str_eq!(
            format!("{:02X?}", *result.get_ref()),
            format!("{:02X?}", expected)
        );

        Ok(())
    }

    #[test]
    fn tre_compressed_multiple_entries_with_data_write() -> Result<()> {
        let hello_data = [
            0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64,
        ];

        let world_data = [
            0x57, 0x6F, 0x72, 0x6C, 0x64, 0x20, 0x48, 0x65, 0x6C, 0x6C, 0x6F,
        ];

        #[rustfmt::skip]
        let expected = [
            // Header
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x02, 0x00, 0x00, 0x00, 
            0x4A, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x30, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x14, 0x00, 0x00, 0x00, 
            0x14, 0x00, 0x00, 0x00,
            // Data
            0x78, 0x9C, 
            0xF3, 0x48, 0xCD, 0xC9, 0xC9, 0x57, 0x08, 0xCF, 0x2F, 0xCA, 0x49, 0x01, 0x00, 
            0x18, 0x0B, 0x04, 0x1D,

            0x78, 0x9C, 
            0x0B, 0xCF, 0x2F, 0xCA, 0x49, 0x51, 0xF0, 0x48, 0xCD, 0xC9, 0xC9, 0x07, 0x00, 
            0x18, 0x83, 0x04, 0x1D,
            // Records
            0xAA, 0x30, 0x7E, 0x52,
            0x0B, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00, 
            0x13, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,

            0xDE, 0x6E, 0xB0, 0xD8,
            0x0B, 0x00, 0x00, 0x00, 
            0x37, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00, 
            0x13, 0x00, 0x00, 0x00, 
            0x0A, 0x00, 0x00, 0x00,
            // Names
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x2E, 0x74, 0x78, 0x74, 0x00,
            0x77, 0x6F, 0x72, 0x6C, 0x64, 0x2E, 0x74, 0x78, 0x74, 0x00,
            // Hashes
            0xED, 0xCF, 0xD4, 0xB0, 0xFA, 0x56, 0xDA, 0x3B, 0x9C, 0x65, 0x20, 0xFB, 0x8B, 0x79, 0x75, 0x61, 
            0xA6, 0x09, 0xD4, 0xBD, 0x27, 0x11, 0x25, 0x4F, 0x25, 0xFC, 0x50, 0xDF, 0x17, 0x0F, 0x2C, 0x70
        ];

        let mut writer = TreWriter::new(
            Cursor::new(Vec::new()),
            TreWriterOptions::builder()
                .record_compression(CompressionMethod::None)
                .name_compression(CompressionMethod::None)
                .build(),
        );
        writer.start_file("hello.txt", CompressionMethod::Zlib)?;
        writer.write_all(&hello_data)?;
        writer.finish_file()?;

        writer.start_file("world.txt", CompressionMethod::Zlib)?;
        writer.write_all(&world_data)?;

        let result = writer.finish()?;
        assert_eq!(result.get_ref().len(), expected.len());
        assert_str_eq!(
            format!("{:02X?}", *result.get_ref()),
            format!("{:02X?}", expected)
        );

        Ok(())
    }
}
