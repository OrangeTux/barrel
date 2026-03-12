use argh::FromArgs;
use color_eyre::{
    Section,
    eyre::{Context, Result, bail},
};
use flate2::{Decompress, FlushDecompress};
use std::{
    fs::{File, canonicalize},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const MAX_FILE_GROUP_COUNT: u8 = 71;

// The Lego Racers disk includes the files data1.cab and data1.hdr.
// The first file is an archive that includes compressed sounds, an .exe
// and more.
//
/// data1.hdr is like an index into data1.cab. It lists metadata of all files, for example
/// the file name, it's compressed and uncompressed size and offset into data1.cab.
///
/// Extract contents of an InstallShield cabinet file
#[derive(FromArgs)]
struct Args {
    /// path to an InstallShield cabinet or file, usually named 'data1.cab'
    #[argh(option, short = 'f')]
    input: String,

    /// location where files are extracted to
    #[argh(option, short = 'o')]
    output: String,
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let args: Args = argh::from_env();

    let mut header_file_path = PathBuf::from(&args.input);

    // The user can provide a path to the .hdr file, or the .cab file,
    // or to the folder that contains these files.
    match header_file_path.extension() {
        Some(extension) => {
            if extension == "cab" {
                header_file_path.set_extension("hdr");
            }
        }
        None => {
            header_file_path = header_file_path.join("data1.hdr");
        }
    }

    let input = canonicalize(&header_file_path)
        .wrap_err("unable to locate the .hdr file")
        .suggestion(format!(
            "verify that {} exists and is readable.",
            &header_file_path.display()
        ))?;

    let mut header_file = BufReader::new(
        File::open(&input)
            .wrap_err("unable to read .hdr file")
            .suggestion(format!(
                "verify that {} exists and is readable.",
                header_file_path.display()
            ))?,
    );

    let mut cabinet_file_path = header_file_path;
    cabinet_file_path.set_extension("cab");

    let mut cabinet_file = BufReader::new(
        File::open(&cabinet_file_path)
            .wrap_err("Unable to locate .cab file")
            .suggestion(format!(
                "verify that {} exists and is readable.",
                &cabinet_file_path.display(),
            ))?,
    );

    let mut field = [0; 20];
    header_file.read_exact(&mut field).wrap_err(
        "Unable to read first 20 bytes of the header. It looks you're input file is way to small.",
    )?;

    let header: CommonHeader = field.into();
    if header.major_version() != 5 {
        bail!(
            "This tool only support InstallShield archives version 5. The provided archive is version {}",
            header.major_version()
        );
    }

    header_file.seek(SeekFrom::Start((header.cab_descriptor_offset()).into()))?;
    skip_n(&mut header_file, 12)?;
    let file_table_offset = take_u32(&mut header_file)?;

    skip_n(&mut header_file, 24)?;
    let entries_in_file_table = take_u32(&mut header_file)?;

    skip_n(&mut header_file, 22)?;

    let directories = list_directories(&mut header_file, &header)?;
    let file_descriptors = list_files(
        &mut header_file,
        &header,
        file_table_offset,
        entries_in_file_table,
        &directories,
    )?;

    for directory in &directories {
        let output = Path::new(&args.output).join(directory);
        std::fs::create_dir_all(output)?;
    }

    for fd in &file_descriptors {
        cabinet_file.seek(SeekFrom::Start(fd.data_offset as u64))?;

        let mut data = Vec::new();
        let mut bytes_left = fd.compressed_size;
        loop {
            let segment_size = take_u16(&mut cabinet_file)? as usize;

            let mut segment: Vec<u8> = vec![0; segment_size];
            cabinet_file.read_exact(&mut segment)?;
            segment.push(b'\0');

            bytes_left -= segment_size as u32 + 2;
            let mut output = vec![0; 1024 * 10];

            let mut decompressor = Decompress::new(false);
            decompressor
                .decompress(&segment, &mut output, FlushDecompress::Finish)
                .wrap_err(format!("Failed to decompress {}", fd.name))?;
            let bytes_written = decompressor.total_out() as usize;
            output.truncate(bytes_written);
            data.append(&mut output);
            if bytes_left == 0 {
                break;
            };
        }

        if data.len() != fd.expanded_size as usize {
            bail!(
                "Failed to decompress {}: the uncompressed should be  {} bytes, but it is {} bytes instead",
                fd.name,
                data.len(),
                fd.expanded_size,
            );
        }

        let output = Path::new(&args.output).join(&fd.directory).join(&fd.name);
        println!("Extracted {output:?}.");
        std::fs::write(output, &data).wrap_err("Failed to create file.")?;
    }

    println!("Extracted {} files.", file_descriptors.len());

    Ok(())
}

/// Extract a list of all directories from the header file.
/// This function moves the cursor of `header_file` forward.
fn list_directories<T>(
    header_file: &mut T,
    header: &CommonHeader,
) -> Result<Vec<String>, std::io::Error>
where
    T: Read + Seek + BufRead,
{
    let mut directories: Vec<String> = Vec::new();

    // Collect the names of all directories.
    for _ in 0..MAX_FILE_GROUP_COUNT {
        let offset = take_u32(header_file)?;
        if offset == 0 {
            continue;
        }
        let corrected_offset = offset + header.cab_descriptor_offset();

        header_file.seek(SeekFrom::Start(corrected_offset.into()))?;
        skip_n(header_file, 4)?;

        // The offset to the directories meta data.
        let descriptor_offset = take_u32(header_file)? + header.cab_descriptor_offset();

        let next_offset = take_u32(header_file)?;

        header_file.seek(SeekFrom::Start(descriptor_offset.into()))?;
        let name_offset = take_u32(header_file)? + header.cab_descriptor_offset();

        header_file.seek(SeekFrom::Start(name_offset.into()))?;
        let name = take_string(header_file)?.trim_end_matches('\0').to_owned();
        directories.push(name);

        if next_offset == 0 {
            break;
        }
    }

    Ok(directories)
}

/// Extract the meta data for all files the header file.
/// This function moves the cursor of `header_file` forward.
fn list_files<T>(
    header_file: &mut T,
    header: &CommonHeader,
    file_table_offset: u32,
    entries_in_file_table: u32,
    directories: &Vec<String>,
) -> Result<Vec<FileDescriptor>, std::io::Error>
where
    T: Read + Seek + BufRead,
{
    let mut file_descriptors = Vec::new();
    for n in 0..entries_in_file_table {
        let n = n + 1;
        let offset = file_table_offset + header.cab_descriptor_offset() + (n * 4);
        header_file.seek(SeekFrom::Start(offset.into()))?;

        let file_offset =
            take_u32(header_file)? + header.cab_descriptor_offset() + file_table_offset;
        header_file.seek(SeekFrom::Start(file_offset.into()))?;
        let name_offset =
            take_u32(header_file)? + header.cab_descriptor_offset() + file_table_offset;
        let current_pos = header_file.stream_position().unwrap();
        header_file.seek(SeekFrom::Start(name_offset.into()))?;
        let name = take_string(header_file)?.trim_end_matches('\0').to_owned();
        header_file.seek(SeekFrom::Start(current_pos))?;

        let directory_index = take_u16(header_file)?;
        let directory = directories
            .get(directory_index as usize)
            .unwrap()
            .to_owned();
        skip_n(header_file, 2)?;

        let flags = take_u16(header_file)?;
        let expanded_size = take_u32(header_file)?;
        let compressed_size = take_u32(header_file)?;

        skip_n(header_file, 20)?;

        let data_offset = take_u32(header_file)?;

        let mut md5 = [0; 0x10];
        header_file.read_exact(&mut md5)?;

        let fd = FileDescriptor {
            volume: n,
            name,
            directory,
            flags,
            expanded_size,
            compressed_size,
            data_offset,
            md5,
        };
        file_descriptors.push(fd);
    }
    Ok(file_descriptors)
}

#[derive(Debug)]
struct CommonHeader(Vec<u8>);

impl CommonHeader {
    fn signature(&self) -> [u8; 4] {
        self.0[0..4].try_into().unwrap()
    }

    fn version(&self) -> [u8; 4] {
        self.0[4..8].try_into().unwrap()
    }

    fn major_version(&self) -> usize {
        // This code is based on https://github.com/twogood/unshield/blob/83dc0f7096fb6237670fe6449589b040028d29a1/lib/libunshield.c#L355.
        (u32::from_le_bytes(self.version()) as usize >> 12) & 0xf
    }

    fn volume_info(&self) -> [u8; 4] {
        self.0[8..12].try_into().unwrap()
    }

    fn cab_descriptor_offset(&self) -> u32 {
        u32::from_le_bytes(self.0[12..16].try_into().unwrap())
    }

    fn cab_descriptor_size(&self) -> u32 {
        u32::from_le_bytes(self.0[16..20].try_into().unwrap())
    }

    fn cab_file_table_offset(&self) -> u32 {
        u32::from_le_bytes(self.0[..20].try_into().unwrap())
    }
}

impl From<[u8; 20]> for CommonHeader {
    fn from(value: [u8; 20]) -> Self {
        let this = Self(value.to_vec());
        assert_eq!(this.signature(), [0x49, 0x53, 0x63, 0x28]);
        this
    }
}

// Read N bytes and discard them.
fn skip_n(reader: &mut impl Read, n: usize) -> Result<(), std::io::Error> {
    let mut buffer = vec![0; n];
    reader.read_exact(&mut buffer)?;
    Ok(())
}

// Read 4 bytes and encode them as u32.
fn take_u16(reader: &mut impl Read) -> Result<u16, std::io::Error> {
    let mut buffer = [0; 2];
    reader.read_exact(&mut buffer)?;
    Ok(u16::from_le_bytes(buffer))
}
// Read 4 bytes and encode them as u32.
fn take_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
    let mut buffer = [0; 4];
    reader.read_exact(&mut buffer)?;
    Ok(u32::from_le_bytes(buffer))
}

/// Read a NULL terminated string.
fn take_string<T>(reader: &mut T) -> Result<String, std::io::Error>
where
    T: Read + Seek + BufRead,
{
    let mut buf = Vec::new();
    reader.read_until(b'\0', &mut buf)?;
    Ok(String::from_utf8(buf).unwrap())
}

#[derive(Debug)]
pub struct FileDescriptor {
    volume: u32,
    name: String,
    directory: String,
    flags: u16,
    expanded_size: u32,
    compressed_size: u32,
    data_offset: u32,
    md5: [u8; 0x10],
}
