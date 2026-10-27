//! Types for reading TRE archives
//!

use binrw::BinRead;
use byteorder::ReadBytesExt;
use indexmap::IndexMap;
use std::{
    borrow::Cow,
    fmt::{self, Debug},
    io::{Read, Seek},
    sync::Arc,
};

use crate::{
    compression::{CompressionMethod, TreBlockReader},
    error::{Error, FileNotFoundError, Result},
    types::{TreHeader, TreRecord},
};

/// A struct for reading an entry from a TRE file
pub struct TreFile<'a, W: Read + Seek> {
    data: Cow<'a, TreFileData>,
    reader: TreBlockReader<'a, W>,
}

impl<'a, W: Read + Seek> Debug for TreFile<'a, W> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "TreFile({:#?})", self.get_metadata())
    }
}

/// Methods for retrieving information on TRE file entries
impl<'a, W: Read + Seek> TreFile<'a, W> {
    /// Get the name of the file
    ///
    /// # Warnings
    ///
    /// It is dangerous to use this name directly when extracting an archive.
    /// It may contain an absolute path (`/etc/shadow`), or break out of the
    /// current directory (`../runtime`). Carelessly writing to these paths
    /// allows an attacker to craft a TRE archive that will overwrite critical
    /// files.
    ///
    pub fn name(&self) -> &str {
        &self.get_metadata().file_name
    }

    /// Get the name of the file, in the raw (internal) byte representation.
    ///
    /// The encoding of this data is currently undefined.
    pub fn name_raw(&self) -> &[u8] {
        &self.get_metadata().file_name_raw
    }

    /// Get the size of the file, in bytes, in the archive
    pub fn compressed_size(&self) -> u64 {
        self.get_metadata().compressed_size
    }

    /// Get the size of the file, in bytes, when uncompressed
    pub fn size(&self) -> u64 {
        self.get_metadata().uncompressed_size
    }

    /// Get the CRC32 hash of the original file
    pub fn crc32(&self) -> u32 {
        self.get_metadata().crc32
    }

    /// Get the starting offset of the data of the compressed file
    pub fn data_start(&self) -> u64 {
        self.get_metadata().data_start
    }

    /// Get the compression method used for this file
    pub fn compression_method(&self) -> CompressionMethod {
        self.get_metadata().compression_method
    }

    fn get_metadata(&self) -> &TreFileData {
        self.data.as_ref()
    }
}

impl<W: Read + Seek> Read for TreFile<'_, W> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

/// Structure representing a TRE file entry.
#[derive(Debug, Clone, Default)]
pub struct TreFileData {
    /// CRC32 checksum
    pub crc32: u32,
    /// Method of compressing the file in the tre
    pub compression_method: CompressionMethod,
    /// Size of the file in the tre
    pub compressed_size: u64,
    /// Size of the file when extracted
    pub uncompressed_size: u64,
    /// Name of the file
    pub file_name: Box<str>,
    /// Raw file name. To be used when file_name was incorrectly decoded.
    pub file_name_raw: Box<[u8]>,
    /// Specifies where the local header of the file starts
    pub header_start: u64,
    /// Specifies where the compressed data of the file starts
    pub data_start: u64,
}

#[derive(Debug)]
pub(crate) struct Shared {
    header: TreHeader,
    files: IndexMap<Box<str>, TreFileData>,
}

/// TRE archive reader
///
/// ```no_run
/// use std::io::prelude::*;
///
/// fn list_tre_contents(reader: impl Read + Seek) -> swg_tre::error::Result<()> {
///     let mut tre = swg_tre::TreArchive::new(reader)?;
///
///     for i in 0..tre.len() {
///         let mut file = tre.by_index(i)?;
///         println!("Filename: {}", file.name());
///         std::io::copy(&mut file, &mut std::io::stdout())?;
///     }
///
///     Ok(())
/// }
/// ```
pub struct TreArchive<R> {
    reader: R,
    shared: Arc<Shared>,
}

impl<R> TreArchive<R> {
    /// Total size of the files in the archive, if it can be known. Doesn't include directories or
    /// metadata.
    pub fn decompressed_size(&self) -> Option<u128> {
        let mut total = 0u128;
        for file in self.shared.files.values() {
            total = total.checked_add(file.uncompressed_size as u128)?;
        }
        Some(total)
    }
}

impl<R: Read + Seek> TreArchive<R> {
    /// Read a TRE archive collecting the files it contains.
    pub fn new(mut reader: R) -> Result<TreArchive<R>> {
        if let Ok(shared) = Self::get_metadata(&mut reader) {
            return Ok(TreArchive {
                reader,
                shared: shared.into(),
            });
        }

        Err(Error::InvalidArchive)
    }

    /// Number of entries contained in this TRE.
    pub fn len(&self) -> usize {
        self.shared.files.len()
    }

    /// Whether this TRE archive contains no entries
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns an iterator over all the file and directory names in this archive.
    pub fn file_names(&self) -> impl Iterator<Item = &str> {
        self.shared.files.keys().map(|s| s.as_ref())
    }

    /// Returns how the records data was compressed.
    pub fn get_record_compression(&self) -> CompressionMethod {
        self.shared.header.record_compression
    }

    /// Returns how the records data was compressed.
    pub fn get_name_compression(&self) -> CompressionMethod {
        self.shared.header.name_compression
    }

    /// Get the index of a file entry by name, if it's present.
    #[inline(always)]
    pub fn index_for_name(&self, name: &str) -> Option<usize> {
        self.shared.files.get_index_of(name)
    }

    /// Get the name of a file entry, if it's present.
    #[inline(always)]
    pub fn name_for_index(&self, index: usize) -> Option<&str> {
        self.shared
            .files
            .get_index(index)
            .map(|(name, _)| name.as_ref())
    }

    /// Search for a file entry by name
    pub fn by_name(&mut self, name: &str) -> Result<TreFile<'_, R>> {
        let Some(index) = self.shared.files.get_index_of(name) else {
            return Err(Error::FileNotFound(FileNotFoundError::Name(
                name.to_owned(),
            )));
        };
        self.by_index(index)
    }

    /// Get a contained file by index
    pub fn by_index(&mut self, file_number: usize) -> Result<TreFile<'_, R>> {
        let (_, data) = self
            .shared
            .files
            .get_index(file_number)
            .ok_or(Error::FileNotFound(FileNotFoundError::Index(file_number)))?;

        Ok(TreFile {
            data: Cow::Borrowed(data),
            reader: TreBlockReader::new(
                &mut self.reader,
                data.data_start,
                data.compressed_size,
                data.compression_method,
            )?,
        })
    }

    /// Unwrap and return the inner reader object
    ///
    /// The position of the reader is undefined.
    pub fn into_inner(self) -> R {
        self.reader
    }

    fn get_records(reader: &mut R, header: &TreHeader) -> Result<Vec<TreRecord>> {
        let mut record_reader = TreBlockReader::new(
            reader,
            header.record_start as u64,
            header.record_compressed as u64,
            header.record_compression,
        )?;

        (0..header.records)
            .map(|_| TreRecord::read(&mut record_reader).map_err(Error::from))
            .collect()
    }

    fn get_names(reader: &mut R, header: &TreHeader) -> Result<Vec<Vec<u8>>> {
        let mut name_reader = TreBlockReader::new(
            reader,
            (header.record_start + header.record_compressed) as u64,
            header.name_compressed as u64,
            header.name_compression,
        )?;

        (0..header.records)
            .map(|_| {
                let mut name_raw: Vec<u8> = Vec::new();
                loop {
                    let char = name_reader.read_u8()?;
                    if char == b'\0' {
                        break;
                    }
                    name_raw.push(char);
                }
                Ok(name_raw)
            })
            .collect()
    }

    fn get_metadata(reader: &mut R) -> Result<Shared> {
        let header = TreHeader::read(reader)?;
        let records = Self::get_records(reader, &header)?;
        let names = Self::get_names(reader, &header)?;

        let mut index_map = IndexMap::with_capacity(header.records as usize);
        records.into_iter().zip(names).for_each(|(r, n)| {
            let file = TreFileData {
                crc32: r.checksum,
                compression_method: r.data_compression,
                compressed_size: r.data_compressed as u64,
                uncompressed_size: r.data_uncompressed as u64,
                data_start: r.data_offset as u64,
                file_name: String::from_utf8_lossy(&n).into(),
                file_name_raw: n.into(),
                ..Default::default()
            };
            index_map.insert(file.file_name.clone(), file);
        });

        Ok(Shared {
            header,
            files: index_map,
        })
    }
}

#[cfg(test)]
mod test {
    use std::io::prelude::*;

    use crate::{error::Result, read::TreArchive};
    use std::io::Cursor;

    #[test]
    fn read_invalid_magic() {
        #[rustfmt::skip]
        let input = [
            0x40, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x00, 0x00, 0x00, 0x00, 
            0x28, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
        ];

        let archive = TreArchive::new(Cursor::new(input));
        assert!(archive.is_err());
    }

    #[test]
    fn read_empty_uncompressed_tre() {
        let input = [
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 0x00, 0x00, 0x00, 0x00, 0x28, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        let archive = TreArchive::new(Cursor::new(input));
        assert!(archive.is_ok());
        assert!(archive.unwrap().is_empty());
    }

    #[test]
    fn read_empty_compressed_tre() {
        let input = [
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 0x00, 0x00, 0x00, 0x00, 0x28, 0x00,
            0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        let archive = TreArchive::new(Cursor::new(input));
        assert!(archive.is_ok());
        assert!(archive.unwrap().is_empty());
    }

    #[test]
    fn read_uncompressed_tre_with_entry() -> Result<()> {
        let input = [
            // Header (36)
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 0x01, 0x00, 0x00, 0x00, 0x2F, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x0A, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, // Data (11)
            0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64,
            // Records (24)
            0x00, 0x00, 0x00, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Names (10)
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x2E, 0x74, 0x78, 0x74, 0x00,
        ];

        let mut archive = TreArchive::new(Cursor::new(input))?;
        assert_eq!(archive.len(), 1);

        let mut buffer = Vec::new();

        let mut file = archive.by_index(0)?;
        assert_eq!(file.data_start(), 36);
        assert_eq!(file.name(), "hello.txt");

        file.reader.read_to_end(&mut buffer)?;
        assert_eq!(
            buffer,
            vec![0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64]
        );

        Ok(())
    }

    #[test]
    fn read_compressed_file_tre_with_entry() -> Result<()> {
        let input = [
            // Header (36)
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 0x01, 0x00, 0x00, 0x00, 0x37, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x0A, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, // Data (19)
            0x78, 0x9C, 0xF3, 0x48, 0xCD, 0xC9, 0xC9, 0x57, 0x08, 0xCF, 0x2F, 0xCA, 0x49, 0x01,
            0x00, 0x18, 0x0B, 0x04, 0x1D, // Records (24)
            0x00, 0x00, 0x00, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Names (10)
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x2E, 0x74, 0x78, 0x74, 0x00,
        ];

        let mut archive = TreArchive::new(Cursor::new(input))?;
        assert_eq!(archive.len(), 1);

        let mut buffer = Vec::new();

        let mut file = archive.by_index(0)?;
        assert_eq!(file.data_start(), 36);
        assert_eq!(file.name(), "hello.txt");

        file.reader.read_to_end(&mut buffer)?;
        assert_eq!(
            buffer,
            vec![0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64]
        );

        Ok(())
    }

    #[test]
    fn read_uncompressed_tre_with_multiple_entries() -> Result<()> {
        let input = [
            // Header (36)
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 0x02, 0x00, 0x00, 0x00, 0x4A, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x14, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, // Data (38)
            0x78, 0x9C, 0xF3, 0x48, 0xCD, 0xC9, 0xC9, 0x57, 0x08, 0xCF, 0x2F, 0xCA, 0x49, 0x01,
            0x00, 0x18, 0x0B, 0x04, 0x1D, 0x78, 0x9C, 0x0B, 0xCF, 0x2F, 0xCA, 0x49, 0x51, 0xF0,
            0x48, 0xCD, 0xC9, 0xC9, 0x07, 0x00, 0x18, 0x83, 0x04, 0x1D, // Records (48)
            0x00, 0x00, 0x00, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00, 0x02, 0x00,
            0x00, 0x00, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x0B, 0x00, 0x00, 0x00, 0x37, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x13, 0x00,
            0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, // Names
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x2E, 0x74, 0x78, 0x74, 0x00, 0x77, 0x6F, 0x72, 0x6C,
            0x64, 0x2E, 0x74, 0x78, 0x74, 0x00,
        ];

        let mut archive = TreArchive::new(Cursor::new(input))?;
        assert_eq!(archive.len(), 2);

        let mut buffer = Vec::new();

        let mut file_first = archive.by_index(0)?;
        assert_eq!(file_first.data_start(), 36);
        assert_eq!(file_first.name(), "hello.txt");

        file_first.reader.read_to_end(&mut buffer)?;
        assert_eq!(
            buffer,
            vec![0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64]
        );
        buffer.clear();

        let mut file_second = archive.by_index(1)?;
        assert_eq!(file_second.data_start(), 55);
        assert_eq!(file_second.name(), "world.txt");

        file_second.reader.read_to_end(&mut buffer)?;
        assert_eq!(
            buffer,
            vec![0x57, 0x6F, 0x72, 0x6C, 0x64, 0x20, 0x48, 0x65, 0x6C, 0x6C, 0x6F]
        );

        Ok(())
    }

    #[test]
    fn read_compressed_file_tre_with_multiple_entries() -> Result<()> {
        let input = [
            // Header (36)
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 0x02, 0x00, 0x00, 0x00, 0x3A, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x14, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, // Data (22)
            0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64, 0x57, 0x6F, 0x72,
            0x6C, 0x64, 0x20, 0x48, 0x65, 0x6C, 0x6C, 0x6F, // Records (48)
            0x00, 0x00, 0x00, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x24, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x0B, 0x00, 0x00, 0x00, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0B, 0x00,
            0x00, 0x00, 0x0A, 0x00, 0x00, 0x00, // Names
            0x68, 0x65, 0x6C, 0x6C, 0x6F, 0x2E, 0x74, 0x78, 0x74, 0x00, 0x77, 0x6F, 0x72, 0x6C,
            0x64, 0x2E, 0x74, 0x78, 0x74, 0x00,
        ];

        let mut archive = TreArchive::new(Cursor::new(input))?;
        assert_eq!(archive.len(), 2);

        let mut buffer = Vec::new();

        let mut file_first = archive.by_index(0)?;
        assert_eq!(file_first.data_start(), 36);
        assert_eq!(file_first.name(), "hello.txt");

        file_first.reader.read_to_end(&mut buffer)?;
        assert_eq!(
            buffer,
            vec![0x48, 0x65, 0x6C, 0x6C, 0x6F, 0x20, 0x57, 0x6F, 0x72, 0x6C, 0x64]
        );
        buffer.clear();

        let mut file_second = archive.by_index(1)?;
        assert_eq!(file_second.data_start(), 47);
        assert_eq!(file_second.name(), "world.txt");

        file_second.reader.read_to_end(&mut buffer)?;
        assert_eq!(
            buffer,
            vec![0x57, 0x6F, 0x72, 0x6C, 0x64, 0x20, 0x48, 0x65, 0x6C, 0x6C, 0x6F]
        );

        Ok(())
    }
}
