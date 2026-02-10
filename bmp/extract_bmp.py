import struct
import hashlib
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
        print(f"yolo {self.out_stride_in_bytes}");
        self.out_image_size_in_byte = self.out_stride_in_bytes * self.height
        print(f"size {self.out_image_size_in_byte}")

        
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

            print(f"{b} {g} {r} 0")

            self.palette[i] = [b, g, r, 0]
        print("----")

    def write(self):
        palette_max_size = 0
        if self.palette_size > 0:
            palette_max_size = 16 if self.bits_per_pixel == 4 else 256

        out_data_offset = 14 + 40 + (4 * palette_max_size)
        
        # File Header
        self.out.write(struct.pack("<2sIHHI", b'BM', out_data_offset + self.out_image_size_in_byte, 0, 0, out_data_offset))
        
        print(f"swag {self.out_image_size_in_byte}")
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
        if len(header) < 4:
            return
        
        # The size of of the compressed bitmap and
        # the size after decompression
        next_decomp_sz, next_comp_sz = struct.unpack("<HH", header)
       
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
        cursor = 0
        
        # First byte 
        self.decompress_buffer[self.decompress_pos] = compressed_data[cursor]
        self.decompress_pos += 1
        cursor += 1
        
        while cursor < len(compressed_data):
            command_byte = compressed_data[cursor]
            cursor += 1

            # Iterate over all bits in the command byte,
            # from MSB to LSB.
            #
            # If a bit is 0, the byte at the cursor is copied
            # into the uncompressed buffer and the cursors position
            # moves by 1 byte.
            for _ in range(8):
                start = cursor
                # If the 7tb bit is set, the next byte encodes 
                # Check if the 7th bit is set.
                # If not, just copy the byte from source to target.
                #
                # The first and second byte after a command byte have some special meaning.
                # These bytes encode an offset (into  the color table?) and a length
                # 
                if (command_byte & 0x80) != 128:
                    self.decompress_buffer[self.decompress_pos] = compressed_data[cursor]
                    self.decompress_pos += 1
                    cursor += 1

                    # Shift the bits one the the left.
                    command_byte = (command_byte << 1) 
                    continue
    
                # 1a_ and 2 are used to calculate the offset. This offset points to a byte 
                # in the uncompressed buffer that must be repeated a bunch of times.
                # 
                # 1b is used determine the repeat count. Only if the repeat count is 18 or more,
                # byte 3 is used. 
                # +--------+--------+--------+ 
                # + 1a  1b |    2   |    3   |  
                # +--------+--------+--------+ 
                _1 = compressed_data[cursor]
                _1a = _1 & 0xF0
                _1b = _1 & 0x0F

                cursor += 1
                _2 = compressed_data[cursor]

                offset = (_1a  << 4) + _2
                cursor += 1
             
                if offset == 0:
                    return

                if offset > self.decompress_pos:
                    raise BmpDumpingException("Invalid compression offset")
                    
                if _1b == 0:
                    # If _1b is 0, byte _3 is used to calculate the
                    # repeat count. Repeat count is 18 the lowest.
                    repeat_count = compressed_data[cursor] + 18
                    cursor += 1
                else:
                    # Since _1b is a value between 1 and 15, the repeat count is a value
                    # between 3 and 17 including including.
                    repeat_count = 18 - _1b
                     
                data = [hex(x) for x in compressed_data[start:cursor]]
                print(data)
                for _ in range(repeat_count):
                    self.decompress_buffer[self.decompress_pos] = \
                        self.decompress_buffer[self.decompress_pos - offset]
                    self.decompress_pos += 1

                command_byte = (command_byte << 1) 
        # self.decompress_pos = 0

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
        print(hashlib.md5(open(output_path,'rb').read()).hexdigest())

    except Exception as e:
        print(f"Error processing file: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()
