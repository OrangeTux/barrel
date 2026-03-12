//! Generates a minimal InstallShield v5 `.hdr` + `.cab` pair for use in tests/CI.
//!
//! The resulting files will unpack to a single file `dir1/file.txt` containing
//! the text `hello\n`.
//!
//! Usage:
//!   cargo run --bin gen_test_cabinet -- --output testdata
//!
//! The generator is intentionally written to be a readable mirror of extract.rs,
//! so that the field offsets in both files stay easy to compare.
//!
//! ## .hdr memory map
//!
//! ```text
//!   0x000 +---------------------------+
//!         | Common header (20 bytes)  |
//!         |   signature "ISc("        |
//!         |   version (major=5)       |
//!         |   volume_info             |
//!         |   cab_descriptor_offset   | --> 0x020
//!         |   cab_descriptor_size     |
//!   0x014 +---------------------------+
//!         | (padding)                 |
//!   0x020 +---------------------------+  <-- cab_descriptor_offset (CAB_DESC = 32)
//!         | Cab descriptor            |
//!         |  +0x00 (skip 12)          |
//!         |  +0x0C file_table_offset  | --> FILE_TABLE_OFFSET (relative to cab_desc)
//!         |  +0x10 (skip 24)          |
//!         |  +0x28 entries_in_table   | = 1
//!         |  +0x2C (skip 22)          |
//!         |  +0x42 dir group slots    | 71 x u32  (values relative to cab_desc)
//!   0x020 + 0x42 + 71*4 = 0x188      |
//!   0x188 +---------------------------+  <-- FILE_TABLE_ABS  (= CAB_DESC + FILE_TABLE_OFFSET)
//!         | File table                |
//!         |   [0] unused (n starts 1) |
//!         |   [1] ptr to file desc    | (relative to cab_desc + file_table_offset)
//!   0x190 +---------------------------+  <-- FILE_DESC_ABS
//!         | File descriptor           |
//!         |   +0x00 name_offset       | (relative to cab_desc + file_table_offset)
//!         |   +0x04 directory_index   |
//!         |   +0x06 padding           |
//!         |   +0x08 flags             |
//!         |   +0x0A expanded_size     |
//!         |   +0x0E compressed_size   |
//!         |   +0x12 (skip 20)         |
//!         |   +0x26 data_offset       |
//!         |   +0x2A md5[16]           |
//!   0x1C0 +---------------------------+  <-- FILE_NAME_ABS  "file.txt\0"
//!   0x1D0 +---------------------------+  <-- DIR_GROUP_ABS  (first non-zero slot points here)
//!         | Dir group descriptor      |
//!         |   +0x00 (skip 4)          |
//!         |   +0x04 descriptor_offset | (relative to cab_desc) --> DIR_DESC_ABS
//!         |   +0x08 next_offset = 0   |
//!   0x1DC +---------------------------+  <-- DIR_DESC_ABS
//!         | Directory descriptor      |
//!         |   +0x00 name_offset       | (relative to cab_desc) --> DIR_NAME_ABS
//!   0x1E8 +---------------------------+  <-- DIR_NAME_ABS  "dir1\0"
//!   0x200 +---------------------------+  end / HDR_SIZE
//! ```

use argh::FromArgs;
use flate2::{Compress, Compression, FlushCompress};
use std::{
    io::{self, Seek, SeekFrom, Write},
    path::Path,
};

/// Generate a minimal InstallShield v5 .hdr + .cab pair for testing
#[derive(FromArgs)]
struct Args {
    /// directory where data1.hdr and data1.cab are written (created if absent)
    #[argh(option, short = 'o')]
    output: String,
}

// ---------------------------------------------------------------------------
// Layout constants
//
// All *_ABS values are absolute byte offsets within the .hdr file.
// They are chosen so that no two regions overlap.
// ---------------------------------------------------------------------------

/// Absolute offset of the cab descriptor.
/// Stored in common header bytes 12-15.
const CAB_DESC: u32 = 0x20;

/// Offset of the file table relative to the cab descriptor.
/// Stored at cab_descriptor + 0x0C.
///
/// The dir-group slots occupy cab_descriptor + 0x42 .. + 0x42 + 71*4 = + 0x168.
/// So the file table must start at or after CAB_DESC + 0x42 + 71*4 = CAB_DESC + 0x168.
/// We pick FILE_TABLE_OFFSET = 0x168 exactly, giving FILE_TABLE_ABS = 0x188.
const FILE_TABLE_OFFSET: u32 = 0x168;

/// Absolute position of the file table.
const FILE_TABLE_ABS: u32 = CAB_DESC + FILE_TABLE_OFFSET;

/// Absolute position of the single file descriptor.
/// Immediately after the two file-table slots (index 0 unused + index 1 pointer = 8 bytes).
const FILE_DESC_ABS: u32 = FILE_TABLE_ABS + 8;

/// Absolute position of the filename string "file.txt\0".
const FILE_NAME_ABS: u32 = FILE_DESC_ABS + 0x3A; // descriptor is 0x2A + 16 (md5) = 0x3A bytes

/// Absolute position of the directory group descriptor.
const DIR_GROUP_ABS: u32 = FILE_NAME_ABS + 0x10; // "file.txt\0" + padding

/// Absolute position of the directory descriptor.
const DIR_DESC_ABS: u32 = DIR_GROUP_ABS + 0x0C; // group is 12 bytes

/// Absolute position of the directory name string "dir1\0".
const DIR_NAME_ABS: u32 = DIR_DESC_ABS + 0x08; // descriptor name_offset (4) + padding (4)

/// Total .hdr file size.
const HDR_SIZE: usize = (DIR_NAME_ABS + 0x10) as usize;

fn main() -> io::Result<()> {
    let args: Args = argh::from_env();
    let out_dir = Path::new(&args.output);
    std::fs::create_dir_all(out_dir)?;

    let raw_content: &[u8] = b"hello\n";

    let cab = build_cab(raw_content)?;
    // compressed_size is the number of cab bytes consumed for this file:
    // a u16 length prefix + the compressed segment data.
    let compressed_size = cab.len() as u32;

    let hdr = build_hdr(raw_content.len() as u32, compressed_size)?;

    let hdr_path = out_dir.join("data1.hdr");
    let cab_path = out_dir.join("data1.cab");

    std::fs::write(&hdr_path, &hdr)?;
    std::fs::write(&cab_path, &cab)?;

    println!("wrote {} ({} bytes)", hdr_path.display(), hdr.len());
    println!("wrote {} ({} bytes)", cab_path.display(), cab.len());

    Ok(())
}

// ---------------------------------------------------------------------------
// .hdr builder
// ---------------------------------------------------------------------------

fn build_hdr(expanded_size: u32, compressed_size: u32) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; HDR_SIZE];
    let mut w = io::Cursor::new(&mut buf);

    // ----- Common header (bytes 0x00 - 0x13) --------------------------------
    //
    // extract.rs reads 20 bytes and constructs a CommonHeader from them.

    w.seek(SeekFrom::Start(0))?;
    // Signature: "ISc("
    w.write_all(&[0x49, 0x53, 0x63, 0x28])?;
    // Version word. extract.rs: (version_u32 >> 12) & 0xf must equal 5.
    write_u32(&mut w, 5 << 12)?;
    // volume_info (unused by extract.rs)
    write_u32(&mut w, 0)?;
    // cab_descriptor_offset
    write_u32(&mut w, CAB_DESC)?;
    // cab_descriptor_size
    write_u32(&mut w, HDR_SIZE as u32 - CAB_DESC)?;

    // ----- Cab descriptor (starts at CAB_DESC = 0x20) -----------------------
    //
    // extract.rs reads the descriptor as follows (positions relative to
    // the seek to cab_descriptor_offset):
    //
    //   seek(CAB_DESC)
    //   skip 12          --> cursor at CAB_DESC + 0x0C
    //   file_table_offset (u32)
    //   skip 24          --> cursor at CAB_DESC + 0x28
    //   entries_in_file_table (u32)
    //   skip 22          --> cursor at CAB_DESC + 0x42
    //   [start of list_directories: reads 71 x u32 slots from here]

    // file_table_offset at CAB_DESC + 0x0C
    w.seek(SeekFrom::Start((CAB_DESC + 0x0C) as u64))?;
    write_u32(&mut w, FILE_TABLE_OFFSET)?;

    // entries_in_file_table at CAB_DESC + 0x28
    w.seek(SeekFrom::Start((CAB_DESC + 0x28) as u64))?;
    write_u32(&mut w, 1)?;

    // ----- Directory group slots (CAB_DESC + 0x42) --------------------------
    //
    // list_directories reads each slot as a u32 `offset`.
    // If non-zero it seeks to `offset + CAB_DESC` (the corrected_offset).
    //
    // We place one non-zero value in slot 0, pointing to DIR_GROUP_ABS:
    //   stored_value = DIR_GROUP_ABS - CAB_DESC

    w.seek(SeekFrom::Start((CAB_DESC + 0x42) as u64))?;
    write_u32(&mut w, DIR_GROUP_ABS - CAB_DESC)?;
    // Remaining 70 slots are zero (already in the buffer), so the loop will
    // `continue` past them and never break early.

    // ----- Directory group descriptor at DIR_GROUP_ABS ----------------------
    //
    // list_directories reads at corrected_offset:
    //   +0x00 skip 4
    //   +0x04 descriptor_offset (u32, relative to CAB_DESC) -> abs DIR_DESC_ABS
    //   +0x08 next_offset (u32) 0 = end of chain

    w.seek(SeekFrom::Start(DIR_GROUP_ABS as u64))?;
    write_u32(&mut w, 0)?; // skipped 4 bytes
    write_u32(&mut w, DIR_DESC_ABS - CAB_DESC)?;
    write_u32(&mut w, 0)?; // no next group

    // ----- Directory descriptor at DIR_DESC_ABS -----------------------------
    //
    // list_directories reads:
    //   +0x00 name_offset (u32, relative to CAB_DESC) -> abs DIR_NAME_ABS

    w.seek(SeekFrom::Start(DIR_DESC_ABS as u64))?;
    write_u32(&mut w, DIR_NAME_ABS - CAB_DESC)?;

    // ----- Directory name at DIR_NAME_ABS -----------------------------------
    w.seek(SeekFrom::Start(DIR_NAME_ABS as u64))?;
    w.write_all(b"dir1\0")?;

    // ----- File table (at FILE_TABLE_ABS = CAB_DESC + FILE_TABLE_OFFSET) ----
    //
    // list_files iterates n from 1..=entries_in_file_table.
    // For each n it reads a pointer from:
    //   file_table_offset + CAB_DESC + n * 4
    //
    // The pointer value, when added to CAB_DESC + file_table_offset, gives
    // the absolute position of the file descriptor:
    //   ptr_value = FILE_DESC_ABS - CAB_DESC - FILE_TABLE_OFFSET
    //
    // The pointer for n=1 lives at FILE_TABLE_ABS + 4.

    w.seek(SeekFrom::Start((FILE_TABLE_ABS + 4) as u64))?;
    write_u32(&mut w, FILE_DESC_ABS - CAB_DESC - FILE_TABLE_OFFSET)?;

    // ----- File descriptor at FILE_DESC_ABS ---------------------------------
    //
    // list_files reads (offsets relative to FILE_DESC_ABS):
    //   +0x00 name_offset    (u32) relative to CAB_DESC + FILE_TABLE_OFFSET
    //   +0x04 directory_index (u16)
    //   +0x06 padding         (skip 2)
    //   +0x08 flags           (u16)
    //   +0x0A expanded_size   (u32)
    //   +0x0E compressed_size (u32)
    //   +0x12 skip 20
    //   +0x26 data_offset     (u32)  byte offset into .cab
    //   +0x2A md5             [u8; 16]

    w.seek(SeekFrom::Start(FILE_DESC_ABS as u64))?;
    // name_offset: relative to CAB_DESC + FILE_TABLE_OFFSET
    write_u32(&mut w, FILE_NAME_ABS - CAB_DESC - FILE_TABLE_OFFSET)?;
    // directory_index = 0  (first and only directory)
    write_u16(&mut w, 0)?;
    // padding
    write_u16(&mut w, 0)?;
    // flags
    write_u16(&mut w, 0)?;
    // expanded_size
    write_u32(&mut w, expanded_size)?;
    // compressed_size
    write_u32(&mut w, compressed_size)?;
    // skip 20 bytes (already zero in the buffer)
    w.seek(SeekFrom::Current(20))?;
    // data_offset into .cab = 0  (our single file starts at byte 0 of the cab)
    write_u32(&mut w, 0)?;
    // md5 (16 bytes, left as zeros)

    // ----- File name at FILE_NAME_ABS ----------------------------------------
    w.seek(SeekFrom::Start(FILE_NAME_ABS as u64))?;
    w.write_all(b"file.txt\0")?;

    Ok(buf)
}

// ---------------------------------------------------------------------------
// .cab builder
// ---------------------------------------------------------------------------

fn build_cab(raw: &[u8]) -> io::Result<Vec<u8>> {
    // extract.rs decompresses each segment with flate2 `Decompress::new(false)`
    // (raw DEFLATE, no zlib wrapper) and appends a NUL byte before calling
    // decompress(). We therefore compress with raw DEFLATE as well (zlib=false).
    let compressed = deflate_compress(raw)?;

    let mut cab = Vec::new();
    // Each segment is prefixed with a u16 little-endian giving the segment's
    // byte length. extract.rs subtracts (segment_size + 2) from bytes_left,
    // so compressed_size stored in the .hdr must equal segment_len + 2.
    let seg_len = compressed.len() as u16;
    cab.extend_from_slice(&seg_len.to_le_bytes());
    cab.extend_from_slice(&compressed);

    Ok(cab)
}

/// Compress `data` using raw DEFLATE (no zlib header/trailer).
fn deflate_compress(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut compressor = Compress::new(Compression::default(), /* zlib_header = */ false);
    // Allocate a buffer that is always large enough.
    let mut output = vec![0u8; data.len() + 64];
    compressor
        .compress(data, &mut output, FlushCompress::Finish)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let written = compressor.total_out() as usize;
    output.truncate(written);
    Ok(output)
}

// ---------------------------------------------------------------------------
// Little-endian write helpers
// ---------------------------------------------------------------------------

fn write_u16(w: &mut impl Write, v: u16) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn write_u32(w: &mut impl Write, v: u32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
