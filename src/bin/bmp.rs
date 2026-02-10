use std::{
    env,
    fs::File,
    io::{ErrorKind, Read, Write},
};

const LENGTH_HEADER: u32 = 14;
const LENGTH_BITMAP_INFO_HEADER: u32 = 40;

fn main() -> Result<(), std::io::Error> {
    let mut args = env::args();
    _ = args.next();
    let file_name = args.next().unwrap();

    let mut file = File::open(&file_name)?;

    let bits_per_pixel = take_u8(&mut file)?;
    let number_of_colors = take_u8(&mut file)?;

    let width_in_px = take_u16(&mut file)?;
    let height_in_px = take_u16(&mut file)?;

    let color_table = create_color_table(&mut file, number_of_colors)?;

    println!("Color table size {}", color_table.len() * 4);
    // Skip 3 bytes we don't need.
    file.read_exact(&mut [0; 3])?;

    let bytes_per_row = (width_in_px * bits_per_pixel as u16) / 8;
    let bitmap = decompress_bitmap(&mut file, bytes_per_row)?;

    // let bitmap = decompress_bitmap(&mut file, bytes_per_row)?;

    let header = BmpHeader {
        bits_per_pixel,
        height: height_in_px as i32,
        width: width_in_px as i32,
        color_table,
        bitmap,
    }
    .to_vec();

    File::create("/tmp/dump.bmp")
        .unwrap()
        .write_all(&header)
        .unwrap();

    Ok(())
}

pub struct BmpHeader {
    bits_per_pixel: u8,
    height: i32,
    width: i32,
    color_table: Vec<[u8; 4]>,
    bitmap: Vec<u8>,
}

impl BmpHeader {
    pub fn to_vec(&mut self) -> Vec<u8> {
        // TODO: remove hard coded numbers
        let offset_bitmap =
            LENGTH_HEADER + LENGTH_BITMAP_INFO_HEADER + (self.color_table.len() * 4) as u32;
        let size = offset_bitmap + self.bitmap.len() as u32;
        let mut header = vec![b'B', b'M'];
        header.append(&mut size.to_le_bytes().to_vec());
        header.append(&mut [0, 0, 0, 0].to_vec());
        header.append(&mut offset_bitmap.to_le_bytes().to_vec());

        // assert_eq!(header.len() as u32, LENGTH_HEADER);
        // assert_eq!(size, 2102);

        // TODO BITMAPINFOHEADER
        let mut bitmap_info_header = vec![];
        bitmap_info_header.append(&mut (LENGTH_BITMAP_INFO_HEADER).to_le_bytes().to_vec());
        bitmap_info_header.append(&mut self.width.to_le_bytes().to_vec());
        bitmap_info_header.append(&mut self.height.to_le_bytes().to_vec());
        // the number of color planes (must be 1)
        bitmap_info_header.append(&mut (1_u16).to_le_bytes().to_vec());

        // the number of bits per pixel,
        bitmap_info_header.append(&mut (self.bits_per_pixel as u16).to_le_bytes().to_vec());

        // the compression method being used. 0 indicates no compression
        bitmap_info_header.append(&mut 0_u32.to_le_bytes().to_vec());

        // the image size
        bitmap_info_header.append(&mut (self.bitmap.len() as u32).to_le_bytes().to_vec());

        // the horizontal resolution of the image. (pixel per metre, signed integer)
        bitmap_info_header.append(&mut (0_u32).to_le_bytes().to_vec());
        // the vertical resolution of the image. (pixel per metre, signed integer)
        bitmap_info_header.append(&mut (0_u32).to_le_bytes().to_vec());

        // the number of colors in the color palette, or 0 to default to 2n
        bitmap_info_header.append(&mut (self.color_table.len() as u32).to_le_bytes().to_vec());

        // the number of important colors used, or 0 when every color is important; generally ignored
        bitmap_info_header.append(&mut (self.color_table.len() as u32).to_le_bytes().to_vec());

        header.append(&mut bitmap_info_header);
        header.append(&mut self.color_table.as_flattened().to_vec());
        for byte in self.bitmap.clone().into_iter() {
            header.push(byte);
        }
        header
    }
}

fn create_color_table(
    reader: &mut impl Read,
    number_of_colors: u8,
) -> Result<Vec<[u8; 4]>, std::io::Error> {
    // The number of entries in the  color table aka number
    // of different colors in the picture.

    // The color table must be padded have either 16 or 256 entries.
    let color_table_entries = { if number_of_colors <= 16 { 16 } else { 256 } };
    let mut color_table = vec![[0, 0, 0, 0]; color_table_entries];
    for i in 0..number_of_colors {
        let b = take_u8(reader)?;
        let g = take_u8(reader)?;
        let r = take_u8(reader)?;
        color_table[i as usize] = [b, g, r, 0];
    }

    Ok(color_table)
}

/// The compressed bitmap starts with 2 u16 encoding the
/// original size in bytes and the number of bytes for the compressed bitmap.
///
/// The rest of the data is the bitmap compressed using a dictionary codec.
/// That means that repeating sequences of bytes are encoded using an offset (where the sequence starts)
/// and a length.
fn decompress_bitmap(
    reader: &mut impl Read,
    bytes_per_row: u16,
) -> Result<Vec<u8>, std::io::Error> {
    let mut bitmap = Vec::new();
    loop {
        let original_size = match take_u16(reader) {
            Ok(size) => size,
            Err(error) => {
                if error.kind() == ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(error);
            }
        };
        let compressed_size = take_u16(reader)?;

        println!(
            "The original bitmap is {original_size} bytes and it is compressed into {compressed_size} bytes"
        );

        let mut compressed_bitmap = vec![0; compressed_size as usize];
        reader.read_exact(&mut compressed_bitmap).unwrap();

        if original_size <= compressed_size {
            bitmap.append(&mut compressed_bitmap);
            break;
        }

        let mut cursor = 0;
        // The first byte is always copied as is.
        bitmap.push(compressed_bitmap[cursor]);
        cursor += 1;

        loop {
            // Return if cursor "walked" over the entire compressed image.
            if cursor >= compressed_bitmap.len() {
                break;
            }

            let mut command_byte = compressed_bitmap[cursor];
            cursor += 1;

            // Iterate over all bits in the command byte.
            for _ in 0..8 {
                if command_byte & 0x80 != 0x80 {
                    bitmap.push(compressed_bitmap[cursor]);
                    cursor += 1;

                    command_byte <<= 1;
                    continue;
                }

                // 1a_ and 2 are used to calculate the offset. This offset points to a sequence
                // of bytes in the final bitmap that must be copied and appended to the end of the bitmap.
                //
                // 1b is used determine the length. Only if the length is 18 or more,
                // byte 3 is used.
                // +--------+--------+--------+
                // + 1a  1b |    2   |    3   |
                // +--------+--------+--------+
                let _1 = compressed_bitmap[cursor];
                let _1a = (_1 & 0xF0) as usize;
                let _1b = (_1 & 0x0F) as usize;

                cursor += 1;
                let _2 = compressed_bitmap[cursor] as usize;

                let offset = (_1a << 4) + _2;
                cursor += 1;

                if offset == 0 {
                    break;
                }

                let length = if _1b == 0 {
                    // If _1b is 0, byte _3 is used to calculate the
                    // repeat count. Repeat count is 18 the lowest.
                    cursor += 1;
                    compressed_bitmap[cursor - 1] as usize + 18
                } else {
                    18 - _1b
                };

                // println!("{:?} - {:?}", bitmap.len(), offset);
                let start = bitmap.len() - offset;
                let end = start + length;

                // Append an existing sequence of bytes to the end of the bitmap.
                for offset in start..end {
                    bitmap.push(bitmap[offset]);
                }

                command_byte <<= 1;
            }
        }
    }

    // BMP files encode the images from bottom to top.
    // So the first row of pixels is encoded using the last bytes in the bitmap.
    // let height_in_px = original_size / bytes_per_row;
    let height_in_px = 32;

    let mut flipped_bitmap = Vec::with_capacity(bitmap.len());
    for y in (0..height_in_px as usize).rev() {
        let start = y * bytes_per_row as usize;
        let end = start + bytes_per_row as usize;
        flipped_bitmap.extend_from_slice(&bitmap[start..end]);
    }

    Ok(flipped_bitmap)
}

fn take_u8(reader: &mut impl Read) -> Result<u8, std::io::Error> {
    let mut buffer = [0; 1];
    reader.read_exact(&mut buffer)?;
    Ok(buffer[0])
}

fn take_u16(reader: &mut impl Read) -> Result<u16, std::io::Error> {
    let mut buffer = [0; 2];
    reader.read_exact(&mut buffer)?;
    Ok(u16::from_le_bytes(buffer))
}
