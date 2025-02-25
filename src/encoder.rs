// MIT License
// Copyright (c) 2025 Gianluca Cannata <gcannata23@gmail.com>
//
// av-gif - A GIF encoder written in Rust
use std::borrow::Cow;

#[derive(Debug)]
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
        use_local_palette: bool,
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
    buffer: Vec<u8>,
    frame_count: u16,
}

impl GifEncoder for GifEncoderState {
    fn process_event<'a>(&mut self, event: GifEvent<'a>) -> Result<(), String> {
        todo!()
    }
}
