import struct
import io
import sys
import argparse
import os

class BmpDumpingException(Exception):
    pass

# --- Constants & Utilities ---
BMP_PALETTE_MAX_SIZE = 256
BMP_DECOMPRESS_BUFFER_SIZE = 1500
CUSTOM_BMP_MASK_BITS_PER_PIXEL = 0x3C
CUSTOM_BMP_FLAG_NO_PALETTE = 0x80
BMP_BI_RGB = 0

def align(value, alignment):
    return (value + alignment - 1) & ~(alignment - 1)

# --- Dumper Class ---
class CustomBmpDumper:
    def __init__(self, in_data, out_stream):
        self.reader = io.BytesIO(in_data)
        self.out = out_stream
        
        self.palette = [[0, 0, 0, 0] for _ in range(BMP_PALETTE_MAX_SIZE)]
        self.decompress_buffer = bytearray(BMP_DECOMPRESS_BUFFER_SIZE)
        self.decompress_pos = 0
        self.decompress_size = 0
        
        self.palette_size = 0
        self.bits_per_pixel = 0
        self.width = 0
        self.height = 0
        self.out_stride_in_bytes = 0
        self.out_image_size_in_byte = 0

    def initialize(self):
        # First 6 bytes include
        # 0 - bits per pixel
        # 1 - palette size (i think this revers to color table)
        # 2,3 - width
        # 4,5 - height
        header_data = self.reader.read(6)
        if len(header_data) < 6:
            raise BmpDumpingException("Header too short")
            
        bpp_raw, pal_size_raw, width, height = struct.unpack("<BBHH", header_data)
        print(f"{bpp_raw} : {pal_size_raw}")
        print(f"{width} x {height}")
        
        self.bits_per_pixel = bpp_raw & CUSTOM_BMP_MASK_BITS_PER_PIXEL
        if self.bits_per_pixel not in [4, 8, 24, 32]:
            raise BmpDumpingException(f"Invalid bitsPerPixel: {self.bits_per_pixel}")
            
        self.width = width
        self.height = height
        self.out_stride_in_bytes = align(self.width * self.bits_per_pixel, 32) // 8
        self.out_image_size_in_byte = self.out_stride_in_bytes * self.height
        
        has_palette = self.bits_per_pixel <= 8 and not (bpp_raw & CUSTOM_BMP_FLAG_NO_PALETTE)
        self.palette_size = pal_size_raw + 1 if has_palette else 0
        print(f"{self.palette_size}")
        self.read_palette()

    def read_palette(self):
        if self.palette_size > BMP_PALETTE_MAX_SIZE:
            raise BmpDumpingException("Palette size too large")
            
        print("----")
        for i in range(self.palette_size):
            pal_data = self.reader.read(3)
            if len(pal_data) < 3: break
            b, g, r = struct.unpack("<BBB", pal_data)

            print(f"{r} {g} {b} 0")

            self.palette[i] = [b, g, r, 0]
        print("----")

    def write(self):
        palette_max_size = 0
        if self.palette_size > 0:
            palette_max_size = 16 if self.bits_per_pixel == 4 else 256

        out_data_offset = 14 + 40 + (4 * palette_max_size)
        
        # File Header
        self.out.write(struct.pack("<2sIHHI", b'BM', out_data_offset + self.out_image_size_in_byte, 0, 0, out_data_offset))
        
        # Info Header
        self.out.write(struct.pack("<IiiHHIIiiII", 
            40, self.width, self.height, 1, self.bits_per_pixel, 
            BMP_BI_RGB, self.out_image_size_in_byte, 0, 0, 
            palette_max_size, palette_max_size
        ))

        # Palette
        for i in range(palette_max_size):
            b, g, r, a = self.palette[i]
            self.out.write(struct.pack("<BBBB", b, g, r, a))

        self.write_data()

    def next_data(self):
        self.decompress_pos = 0
        header = self.reader.read(4)
        if len(header) < 4: return
        
        next_decomp_sz, next_comp_sz = struct.unpack("<HH", header)
        print("next_data")
       
        if next_decomp_sz <= next_comp_sz:
            if next_decomp_sz > len(self.decompress_buffer):
                raise BmpDumpingException("Decompress buffer overflow")
            self.decompress_size = next_decomp_sz
            self.decompress_buffer[:self.decompress_size] = self.reader.read(next_decomp_sz)
        else:
            self.decompress_size = next_decomp_sz
            self.decompress_next(next_comp_sz)

    def decompress_next(self, compressed_size):
        """" Read the compressed image from the source file and decompress it
        and write it to self.decompress_buffer
        """
        print(f"read image: {compressed_size}")

        # This seem to be the entire image data
        compressed_data = self.reader.read(compressed_size)
        c_idx = 0
        
        # First byte 
        self.decompress_buffer[self.decompress_pos] = compressed_data[c_idx]
        self.decompress_pos += 1
        c_idx += 1
        
        commands_end = False
        # Iterate over every byte of the image data, 
        # or when commands_end is False
        while not commands_end and c_idx < len(compressed_data):
            command_byte = compressed_data[c_idx]
            c_idx += 1
            # Why 8?
            for _ in range(8):
                # Check if the 7th bit is set.
                # If not, just copy the byte from source to target.
                #
                # The first and second byte after a command byte have some special meaning.
                #
                # After uncompressing, I think the each byte 
                # 
                # 
                if command_byte & 0x80:
                    fle_byte = compressed_data[c_idx]
                    c_idx += 1
                    lower_nibble = fle_byte & 0x0F
                    higher_nibble = fle_byte & 0xF0
                    # The second byte after a cmd byte is some offset.
                    rev_offset = (higher_nibble << 4) | compressed_data[c_idx]
                    print(f"command byte: {command_byte},  fle_byte: {hex(fle_byte)}, rev offset: {rev_offset}, {compressed_data[c_idx]}")
                    c_idx += 1
                 
                    if rev_offset == 0:
                        commands_end = True
                        break
                    if rev_offset > self.decompress_pos:
                        raise BmpDumpingException("Invalid compression offset")
                        
                    repeat_count = -(lower_nibble - 18) if lower_nibble else compressed_data[c_idx] + 18
                    print(f"{lower_nibble} repeat count: {repeat_count}")
                    if not lower_nibble: c_idx += 1
                        
                    # Copy by
                    for _ in range(repeat_count):
                        print(hex(self.decompress_buffer[self.decompress_pos - rev_offset]))
                        self.decompress_buffer[self.decompress_pos] = \
                            self.decompress_buffer[self.decompress_pos - rev_offset]
                        self.decompress_pos += 1
                else:
                    self.decompress_buffer[self.decompress_pos] = compressed_data[c_idx]
                    self.decompress_pos += 1
                    c_idx += 1

                command_byte = (command_byte << 1) 
                if commands_end: break
        self.decompress_pos = 0

    def write_data(self):
        # Width of a line in bytes.
        scan_line_len = align(self.width * self.bits_per_pixel, 8) // 8
        image_data = bytearray(scan_line_len * self.height)
        
        for y in range(self.height):
            remaining = scan_line_len
            row_start = scan_line_len * (self.height - y - 1)
            offset = 0
            while remaining > 0:
                if self.decompress_pos >= self.decompress_size:
                    self.next_data()
                chunk = min(remaining, self.decompress_size - self.decompress_pos)
                if chunk > 0:
                    image_data[row_start + offset : row_start + offset + chunk] = \
                        self.decompress_buffer[self.decompress_pos : self.decompress_pos + chunk]
                    self.decompress_pos += chunk
                    offset += chunk
                    remaining -= chunk

        padding = b'\x00' * (self.out_stride_in_bytes - scan_line_len)
        for y in range(self.height):
            self.out.write(image_data[y * scan_line_len : (y + 1) * scan_line_len])
            self.out.write(padding)

# --- Main Entry Point ---
def main():
    parser = argparse.ArgumentParser(description="Convert custom compressed BMP files to standard BMP.")
    parser.add_argument("input", help="Path to the custom BMP file")
    parser.add_argument("-o", "--output", help="Path to save the output BMP (optional)")
    
    args = parser.parse_args()

    if not os.path.exists(args.input):
        print(f"Error: File '{args.input}' not found.")
        sys.exit(1)

    # Determine output filename if not provided
    output_path = args.output if args.output else os.path.splitext(args.input)[0] + "_dumped.bmp"

    try:
        with open(args.input, "rb") as f_in:
            data = f_in.read()

        if data[:2] == b'BM':
            print(f"Standard BMP detected. Copying to {output_path}...")
            with open(output_path, "wb") as f_out:
                f_out.write(data)
            return

        print(f"Custom BMP detected. Processing {args.input}...")
        with open(output_path, "wb") as f_out:
            dumper = CustomBmpDumper(data, f_out)
            dumper.initialize()
            dumper.write()
            
        print(f"Success! Saved to {output_path}")

    except Exception as e:
        print(f"Error processing file: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()
