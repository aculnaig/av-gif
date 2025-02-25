// MIT License
// Copyright (c) 2025 Gianluca Cannata <gcannata23@gmail.com>
//
// av-gif - A GIF encoder written in Rust
use std::borrow::Cow;

use crate::lzw::LzwEncoder;

#[derive(Debug, PartialEq)]
pub enum DisposalMethod {
    None,       // 0 - No disposal specified
    Keep,       // 1 - Keep previous image
    Background, // 2 - Restore background color
    Previous,   // 3 - Restore previous frame
}

#[derive(Debug)]
pub enum GifEvent<'a> {
    StartGif {
        width: u16,
        height: u16,
        global_palette: Option<Cow<'a, [[u8; 3]]>>, // Borrowed or owned palette
        background_color_index: u8,
        loop_count: Option<u16>,
    },
    StartFrame {
        delay: u16,
        disposal_method: DisposalMethod,
        local_palette: Option<Cow<'a, [[u8; 3]]>>,
        transparent_color_index: Option<u8>,
    },
    WriteImageChunk {
        data: Cow<'a, [u8]>, // LZW-compressed chunk
    },
    FlushFrame, // Optional event to force buffer writing before EndFrame
    EndFrame,
    EndGif,
}

pub trait GifEncoder {
    fn process_event<'a>(&mut self, event: GifEvent<'a>) -> Result<(), String>;
}

// State Transitions
//
// Current State | Event           | Next State    | Notes
// Idle          | StartGif        | WritingHeader | Initialized GIF encoding
// WritingHeader | StartFrame      | WritingFrame  | Begin a new frame
// WritingFrame  | WriteImageChunk | WritingFrame  | Accept LZW-compressed image data
// WritingFrame  | FlushFrame      | FlushingFrame | Force writing buffered data
// WritingFrame  | EndFrame        | WritingHeader | End current frame
// FlushingFrame | EndFrame        | WritingHeader | Ensure all data is written before moving on
// WritingHeader | EndGif          | Finalizing    | Close GIF stream
// Finalizing    | (Completed)     | Done          | GIF is fully encoded
#[derive(Debug, PartialEq)]
enum EncoderState {
    Idle,          // Before 'StartGif'
    WritingHeader, // Writing GIF header and global palette
    WritingFrame,  // Encoding a frame (accepts image data)
    FlushingFrame, // Ensuring all frame data is written
    Finalizing,    // Writing GIF trailer
    Done,          // GIF is finalized
}

struct GifEncoderState {
    state: EncoderState,
    writer: GifWriter,
    lzw_encoder: LzwEncoder,
    frame_count: u16,
    width: u16,
    height: u16,
    // Store loop count for animated GIFs
    loop_count: Option<u16>,
}

impl GifEncoder for GifEncoderState {
    fn process_event<'a>(&mut self, event: GifEvent<'a>) -> Result<(), String> {
        match (&self.state, event) {
            (
                EncoderState::Idle,
                GifEvent::StartGif {
                    width,
                    height,
                    background_color_index,
                    global_palette,
                    loop_count,
                },
            ) => {
                self.state = EncoderState::WritingHeader;
                self.writer.write_gif_header(
                    width,
                    height,
                    background_color_index,
                    global_palette.as_deref(),
                    loop_count,
                );

                self.width = width;
                self.height = height;
                self.loop_count = loop_count;

                Ok(())
            }

            (
                EncoderState::WritingHeader,
                GifEvent::StartFrame {
                    delay,
                    disposal_method,
                    local_palette,
                    transparent_color_index,
                },
            ) => {
                self.state = EncoderState::WritingFrame;

                // Write Graphic Color Extension
                self.writer.write_graphic_control_exension(
                    disposal_method,
                    delay,
                    transparent_color_index,
                );

                // Write Image Descriptor
                self.writer.write_image_descriptor(
                    0,
                    0,
                    self.width,
                    self.height,
                    local_palette.as_deref(),
                );

                Ok(())
            }

            (EncoderState::WritingFrame, GifEvent::WriteImageChunk { data }) => {
                self.lzw_encoder.encode_chunk(data.as_ref());
                Ok(())
            }

            (EncoderState::WritingFrame, GifEvent::FlushFrame) => {
                self.lzw_encoder.finalize();
                let compressed_data = self.lzw_encoder.get_encoded_data();

                // GIF stores image data in blocks (each max 255 bytes)
                for chunk in compressed_data.chunks(255) {
                    // Block size
                    self.writer.buffer.push(chunk.len() as u8);
                    self.writer.buffer.extend_from_slice(chunk);
                }

                // Block terminator
                self.writer.buffer.push(0x00);

                self.state = EncoderState::FlushingFrame;
                Ok(())
            }

            (EncoderState::FlushingFrame, GifEvent::EndFrame)
            | (EncoderState::WritingFrame, GifEvent::EndFrame) => {
                self.state = EncoderState::WritingHeader;
                self.writer.write_frame_trailer();
                self.frame_count += 1;
                Ok(())
            }

            (EncoderState::WritingHeader, GifEvent::EndGif) => {
                self.state = EncoderState::Finalizing;
                self.writer.write_gif_trailer();
                self.state = EncoderState::Done;
                Ok(())
            }

            _ => Err("Invalid event for current state".to_string()),
        }
    }
}

pub struct GifWriter {
    buffer: Vec<u8>,
}

impl GifWriter {
    pub fn new() -> Self {
        GifWriter { buffer: Vec::new() }
    }

    pub fn get_encoded_data(&self) -> &[u8] {
        &self.buffer
    }

    pub fn write_gif_header(
        &mut self,
        width: u16,
        height: u16,
        background_index: u8,
        global_palette: Option<&[[u8; 3]]>,
        loop_count: Option<u16>,
    ) {
        // GIF signature + version
        self.buffer.extend_from_slice(b"GIF89a");

        // Logical Screen Descriptor (LSD)
        self.buffer.extend_from_slice(&width.to_le_bytes());
        self.buffer.extend_from_slice(&height.to_le_bytes());

        // Global Color Table Flag (1 bit) | Color Resolution (3 bits) | Sort Flag (1 bit) | Size of Global Color Table (3 bits)
        let mut packed_fields = 0u8;
        if let Some(palette) = global_palette {
            packed_fields |= 0b1000_0000; // Set GCT flag
            let gct_size = ((palette.len() as u8).next_power_of_two().trailing_zeros() - 1) as u8;
            packed_fields |= gct_size & 0b0000_0111; // Store GCT size
        }

        self.buffer.push(packed_fields);

        // Background color index
        self.buffer.push(background_index);

        // Pixel Aspect Ratio (0 = default aspect ratio)
        self.buffer.push(0);

        // Write global palette if present
        if let Some(palette) = global_palette {
            for color in palette {
                self.buffer.extend_from_slice(color);
            }
        }

        // Write loop count if this is an animated GIF
        if let Some(loop_count) = loop_count {
            // Netscape Extensions (looping behaviour)
            self.buffer.push(0x21); // Exntesion Introducer
            self.buffer.push(0xFF); // Application Extension Label
            self.buffer.push(0x0B); // Block Size
            self.buffer.extend_from_slice(b"NETSCAPE2.0");
            self.buffer.push(0x03); // Subblock size
            self.buffer.push(0x01); // Loop type (1 = loop)
            self.buffer.extend_from_slice(&loop_count.to_le_bytes()); // Loop count
            self.buffer.push(0x00); // Block terminator
        }
    }

    pub fn write_graphic_control_exension(
        &mut self,
        disposal_method: DisposalMethod,
        delay: u16,
        transparent_color_index: Option<u8>,
    ) {
        self.buffer.push(0x21); // Extension Introducer
        self.buffer.push(0xF9); // Graphic Control Label
        self.buffer.push(0x04); // Block Size (always 4 bytes)

        // Packed Fields: Disposal method (3 bits) | User Input Flag (1 bit) | Transparent Color Flag (1 bit)
        let mut packed_fields = 0u8;
        match disposal_method {
            DisposalMethod::None => {}
            DisposalMethod::Keep => packed_fields |= 0b0000_0100,
            DisposalMethod::Background => packed_fields |= 0b0000_1000,
            DisposalMethod::Previous => packed_fields |= 0b0000_1100,
        }

        if transparent_color_index.is_some() {
            packed_fields |= 0b0000_0001;
        }

        self.buffer.push(packed_fields);

        // Frame delay
        self.buffer.extend_from_slice(&delay.to_le_bytes());

        // Transpared color index (or 0 if unused)
        self.buffer.push(transparent_color_index.unwrap_or(0));

        // Block Terminator
        self.buffer.push(0x00);
    }

    pub fn write_image_descriptor(
        &mut self,
        left: u16,
        top: u16,
        width: u16,
        height: u16,
        local_palette: Option<&[[u8; 3]]>,
    ) {
        self.buffer.push(0x2C); // Image Separator

        // Image Position (2 bytes each)
        self.buffer.extend_from_slice(&left.to_le_bytes());
        self.buffer.extend_from_slice(&top.to_le_bytes());

        // Image Size (2 bytes each)
        self.buffer.extend_from_slice(&width.to_le_bytes());
        self.buffer.extend_from_slice(&height.to_le_bytes());

        // Packed Fields: Local Color Table Flag (1 bit) | Interlace Flag (1 bit) | Sort Flag (1 bit) | Size of Local Color Table (3 bits)
        let mut packed_fields = 0u8;
        if let Some(palette) = local_palette {
            packed_fields |= 0b1000_0000; // Set LCT flag
            let lct_size = ((palette.len() as u8).next_power_of_two().trailing_zeros() - 1) as u8;
            packed_fields |= lct_size & 0b0000_0111; // Store LCT size
        }

        self.buffer.push(packed_fields);

        // Write local palette if present
        if let Some(palette) = local_palette {
            for color in palette {
                self.buffer.extend_from_slice(color);
            }
        }
    }

    pub fn write_frame_trailer(&mut self) {
        // Frame Trailer
        self.buffer.push(0x00);
    }

    pub fn write_gif_trailer(&mut self) {
        // GIF Trailer (End of File)
        self.buffer.push(0x3B);
    }
}
