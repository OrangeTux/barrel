// This script is heavily based on https://github.com/JrMasterModelBuilder/JAM-Extractor/blob/master/JAMExtractor.py
use std::{
    fs::{File, canonicalize},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use argh::FromArgs;

/// Extract a JAM archive. The archive contains files and folders that include
/// the assets of the game.
#[derive(FromArgs)]
struct Args {
    /// input
    #[argh(option, short = 'f')]
    input: String,

    /// location of the transcoded BMP file.
    #[argh(option, short = 'o')]
    output: String,
}

fn main() -> Result<(), std::io::Error> {
    let args: Args = argh::from_env();
    let input = canonicalize(Path::new(&args.input)).inspect_err(|err| {
        eprintln!(
            "Failed to locate input {}, does it exists?. The error is {err:?}",
            args.input
        )
    })?;
    let output = Path::new(&args.output);

    let mut file = BufReader::new(
        File::open(&input)
            .inspect_err(|err| eprintln!("Failed to read input {}: {err:?}", input.display()))?,
    );
    std::fs::create_dir_all(output).inspect_err(|err| {
        eprintln!("Failed to created directory {}: {err:?}", output.display())
    })?;

    let mut header = [0; 4];
    file.read_exact(&mut header).inspect_err(|err| eprintln!("Failed to read the first 4 bytes of the JAM archive. Is it really an archive? The error is {err:?}"))?;
    if header != [b'L', b'J', b'A', b'M'] {
        panic!(
            "The JAM archive at {} does seem to be a valid archive since it does _not_ start with the bytes LJAM.",
            input.display()
        );
    }

    extract(&mut file, output.to_path_buf())?;
    println!(
        "Extracted {} to {}.",
        input.display(),
        canonicalize(output)?.display(),
    );

    Ok(())
}

// Extract a JAM file to the given destination.
//
// A JAM file starts with a 4 byte header containing the ASCII characters LJAM.
//
// Then it starts with the contents of a folder. A folder contains 0 or more files
// and 0 or more folders.
//
//
// Next any subfolders are encoded. The first 4 bytes encode the number of folders.
// Each folder takes up 14 bytes: 12 to encode the file name and 4 to encode the offset
// of the folder's content.
fn extract<T>(reader: &mut T, destination: PathBuf) -> Result<(), std::io::Error>
where
    T: Read + Seek + BufRead,
{
    // Retrieve the amount of files in the folder.
    let amount_of_files = take_u32(reader)?;

    for _ in 0..amount_of_files {
        extract_file(reader, destination.clone())?;
    }

    let amount_of_folders = take_u32(reader)?;
    for _ in 0..amount_of_folders {
        let mut folder_name = [0; 12];
        reader.read_exact(&mut folder_name)?;
        let folder_name = parse_string(folder_name);

        let offset = take_u32(reader)?;

        let mut destination = destination.clone();
        destination.push(&folder_name);

        std::fs::create_dir_all(&destination).inspect_err(|err| {
            panic!(
                "Failed to created directory {}: {err:?}",
                destination.display()
            )
        })?;
        // println!("{} - {}", &destination.display(), offset);

        let cursor = reader.stream_position()?;
        reader.seek(SeekFrom::Start(offset.into()))?;
        extract(reader, destination)?;
        reader.seek(SeekFrom::Start(cursor))?;
    }

    Ok(())
}

// Decode a file and write it to the given destination.
//
// A file is encoded using 16 bytes.
// * 12 to encode the file name
// * 4 to encode the offset
// * 4 to encode the size of the file's content
fn extract_file<T>(reader: &mut T, destination: PathBuf) -> Result<(), std::io::Error>
where
    T: Read + Seek + BufRead,
{
    let mut file_name = [0; 12];
    reader.read_exact(&mut file_name)?;

    let file_name = parse_string(file_name);
    let offset = take_u32(reader)?;
    let size = take_u32(reader)?;

    let mut destination = destination.clone();
    destination.push(&file_name);

    // Copy the file's content from the offset.
    let cursor = reader.stream_position()?;
    reader.seek(SeekFrom::Start(offset.into()))?;
    let mut data = Vec::new();
    for _ in 0..size {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        data.push(byte[0])
    }

    std::fs::write(&destination, &data)?;

    // println!("{} - {} - {}", destination.display(), offset, size);
    reader.seek(SeekFrom::Start(cursor))?;
    Ok(())
}

// Read 4 bytes and encode them as u32.
fn take_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
    let mut buffer = [0; 4];
    reader.read_exact(&mut buffer)?;
    Ok(u32::from_le_bytes(buffer))
}

// Decode the raw bytes into a String.
//
// File names and folders have a fixes size of 12 bytes and are separated by a comma
// If names use less bytes they're padded with zero bytes.
fn parse_string(value: [u8; 12]) -> String {
    value
        .into_iter()
        .filter_map(|byte| {
            // Folder names are always 12 bytes and padded
            // with empty bytes if needed. They're split by a comma.
            if byte == b'\0' {
                return None;
            }
            char::from_u32(byte as u32)
        })
        .collect()
}
