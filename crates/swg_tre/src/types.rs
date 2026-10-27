//! Base types for structure of TRE file.

use crate::compression::CompressionMethod;
use binrw::{BinRead, BinWrite};

/// TRE file header
///
/// Defines the header of the TRE file which always starts with "TREE" and then a version (in this case "0005").
/// All data is stored in little endian format
#[derive(BinRead, BinWrite, Debug, Copy, Clone, PartialEq)]
#[brw(magic = b"EERT5000", little)]
pub struct TreHeader {
    /// The number of records stored in the file
    pub records: u32,

    /// The offset from the beginning of the file where the record metadata starts
    pub record_start: u32,

    /// The compression type used for compressing the record metadata block
    pub record_compression: CompressionMethod,

    /// The size in the file for the compressed record metadata block
    pub record_compressed: u32,

    /// The compression type used for compressing the block of file names
    pub name_compression: CompressionMethod,

    /// The size of the name block after compression
    pub name_compressed: u32,

    /// The size of the name block before compression
    #[allow(dead_code)]
    pub name_uncompressed: u32,
}

impl Default for TreHeader {
    fn default() -> Self {
        Self {
            records: Default::default(),
            record_start: 36,
            record_compression: Default::default(),
            record_compressed: Default::default(),
            name_compression: Default::default(),
            name_compressed: Default::default(),
            name_uncompressed: Default::default(),
        }
    }
}

/// TRE file record
///
/// Defines an entry in the TRE file
#[derive(BinRead, BinWrite, Debug, Default, Copy, Clone, PartialEq)]
#[brw(little)]
pub struct TreRecord {
    /// A [`crc::CRC_32_BZIP2`] checksum of the record's name
    pub checksum: u32,

    /// The size of the data for this record before compression
    pub data_uncompressed: u32,

    /// The offset to the data for this record from the start of the file
    pub data_offset: u32,

    /// The compression type used to compress this record's data
    pub data_compression: CompressionMethod,

    /// The size of this record's data after compression
    pub data_compressed: u32,

    /// The offset from the start of the name block for this record's name
    #[allow(dead_code)]
    pub name_offset: u32,
}

#[cfg(test)]
mod test {
    use std::io::Cursor;

    use binrw::BinRead;
    use binrw::BinWrite;
    use pretty_assertions::assert_eq;

    use crate::compression::CompressionMethod;
    use crate::error::Result;
    use crate::types::TreHeader;
    use crate::types::TreRecord;

    #[test]
    fn read_uncompressed_header() -> Result<()> {
        #[rustfmt::skip]
        let mut input = Cursor::new(vec![
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
        ]);

        let expected = TreHeader {
            record_start: 36,
            record_compression: CompressionMethod::None,
            name_compression: CompressionMethod::None,
            ..Default::default()
        };

        assert_eq!(TreHeader::read(&mut input)?, expected);

        Ok(())
    }

    #[test]
    fn read_compressed_header() -> Result<()> {
        #[rustfmt::skip]
        let mut input = Cursor::new(vec![
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
        ]);

        let expected = TreHeader {
            record_start: 36,
            record_compression: CompressionMethod::Zlib,
            name_compression: CompressionMethod::Zlib,
            ..Default::default()
        };

        assert_eq!(TreHeader::read(&mut input)?, expected);

        Ok(())
    }

    #[test]
    fn write_uncompressed_header() -> Result<()> {
        #[rustfmt::skip]
        let expected: Vec<u8> = vec![
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
        ];

        let header = TreHeader {
            record_start: 36,
            record_compression: CompressionMethod::None,
            name_compression: CompressionMethod::None,
            ..Default::default()
        };

        let mut actual = Vec::new();
        header.write(&mut Cursor::new(&mut actual))?;

        assert_eq!(actual, expected);

        Ok(())
    }

    #[test]
    fn write_compressed_header() -> Result<()> {
        #[rustfmt::skip]
        let expected: Vec<u8> = vec![
            0x45, 0x45, 0x52, 0x54, 0x35, 0x30, 0x30, 0x30, 
            0x00, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x02, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00,
        ];

        let header = TreHeader {
            record_start: 36,
            record_compression: CompressionMethod::Zlib,
            name_compression: CompressionMethod::Zlib,
            ..Default::default()
        };

        let mut actual = Vec::new();
        header.write(&mut Cursor::new(&mut actual))?;

        assert_eq!(actual, expected);

        Ok(())
    }

    #[test]
    fn read_record() -> Result<()> {
        #[rustfmt::skip]
        let mut input = Cursor::new(vec![
            0x00, 0x00, 0x00, 0x00, 
            0x0B, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x0B, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00
        ]);

        let expected = TreRecord {
            data_uncompressed: 11,
            data_offset: 36,
            data_compression: CompressionMethod::None,
            data_compressed: 11,
            ..Default::default()
        };

        assert_eq!(TreRecord::read(&mut input)?, expected);

        Ok(())
    }

    #[test]
    fn write_record() -> Result<()> {
        #[rustfmt::skip]
        let expected = vec![
            0x00, 0x00, 0x00, 0x00, 
            0x0B, 0x00, 0x00, 0x00, 
            0x24, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00, 
            0x0B, 0x00, 0x00, 0x00, 
            0x00, 0x00, 0x00, 0x00
        ];

        let record = TreRecord {
            data_uncompressed: 11,
            data_offset: 36,
            data_compression: CompressionMethod::None,
            data_compressed: 11,
            ..Default::default()
        };

        let mut actual = Vec::new();
        record.write(&mut Cursor::new(&mut actual))?;

        assert_eq!(actual, expected);

        Ok(())
    }
}
