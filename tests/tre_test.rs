use std::io::{Cursor, Read, Write};
use swg_tre::{write::TreWriterOptions, CompressionMethod, TreArchive, TreWriter};

#[test]
fn test_create_and_read_tre() {
    let mut buffer = Cursor::new(Vec::new());

    // Create a valid SWG TRE archive with two files
    {
        let options = TreWriterOptions::builder()
            .name_compression(CompressionMethod::None)
            .record_compression(CompressionMethod::None)
            .build();
        let mut writer = TreWriter::new(&mut buffer, options);

        writer.start_file("sample/texture.dds", CompressionMethod::Zlib).unwrap();
        writer.write_all(b"DDS|FAKE_TEXTURE_PAYLOAD_DATA_12345").unwrap();

        writer.start_file("appearance/mesh.mgn", CompressionMethod::None).unwrap();
        writer.write_all(b"MGN|FAKE_MESH_IFF_RECORD_DATA_67890").unwrap();

        writer.finish().unwrap();
    }

    let bytes = buffer.into_inner();
    assert!(!bytes.is_empty());

    // Read back archive using TreArchive
    let mut archive = TreArchive::new(Cursor::new(bytes.clone())).expect("Valid TRE");
    assert_eq!(archive.len(), 2);

    let names: Vec<&str> = archive.file_names().collect();
    assert!(names.contains(&"sample/texture.dds"));
    assert!(names.contains(&"appearance/mesh.mgn"));

    let mut file = archive.by_name("sample/texture.dds").expect("Found file");
    assert_eq!(file.compression_method(), CompressionMethod::Zlib);
    let mut content = Vec::new();
    file.read_to_end(&mut content).unwrap();
    assert_eq!(content, b"DDS|FAKE_TEXTURE_PAYLOAD_DATA_12345");

    let mut file2 = archive.by_name("appearance/mesh.mgn").expect("Found file 2");
    assert_eq!(file2.compression_method(), CompressionMethod::None);
    let mut content2 = Vec::new();
    file2.read_to_end(&mut content2).unwrap();
    assert_eq!(content2, b"MGN|FAKE_MESH_IFF_RECORD_DATA_67890");
}
