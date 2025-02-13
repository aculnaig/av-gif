use std::io::SeekFrom;

use av_data::{params::{CodecParams, MediaKind, VideoInfo}, rational::Rational64};
use av_format::{buffer::Buffered, common::GlobalInfo, demuxer::{Demuxer, Event}, error::Error, stream::Stream};
use av_format::error::Result;
use nom::{branch::alt, bytes::streaming::{tag, take}, number::{le_u8, streaming::le_u16}, IResult, Parser};

pub struct GifDemuxer {
    pub screen_width: u16,
    pub screen_height: u16,
    pub packed_fields: u8,
    pub background_color_index: u8,
    pub pixel_aspect_ratio: u8,
    pub global_color_table: Vec<u8>,
    pub frames: Vec<GifFrame>,
}

impl GifDemuxer {
    pub fn new() -> Self {
        Self {
            screen_width: 0,
            screen_height: 0,
            packed_fields: 0,
            background_color_index: 0,
            pixel_aspect_ratio: 0,
            global_color_table: Vec::new(),
            frames: Vec::new(),
        }
    }

    // GIF Header parsing
    pub fn parse_header(input: &[u8]) -> IResult<&[u8], ()> {
        // GIF signature ("GIF") and version ("87a" or "89a")
        let (input, _) = ((tag("GIF"), alt((tag("87a"), tag("89a"))))).parse(input)?;
        Ok((input, ()))
    }

    // Logical Screen Descriptor parsing
    pub fn parse_logical_screen_descriptor(input: &[u8]) -> IResult<&[u8], (u16, u16, u8, u8, u8)> {
        let (input, width) = le_u16(input)?;
        let (input, height) = le_u16(input)?;
        let (input, packed_fields) = le_u8().parse(input)?;
        let (input, background_color_index) = le_u8().parse(input)?;
        let (input, pixel_aspect_ratio) = le_u8().parse(input)?;
        Ok((input, (width, height, packed_fields, background_color_index, pixel_aspect_ratio)))
    }

    // Global Color Table parsing
    pub fn parse_global_color_table(input: &[u8], packed_fields: u8) -> IResult<&[u8], Vec<u8>> {
        if packed_fields & 0x80 != 0 {
            let size: u32 = 3 * (1 << ((packed_fields & 0x07) + 1));
            let (input, table) = take(size)(input)?;
            return Ok((input, table.to_vec()))
        }

        Ok((input, Vec::new()))
    }

    // Image Descriptor parsing
    pub fn parse_image_descriptor(input: &[u8]) -> IResult<&[u8], GifFrame> {
        let (input, _) = tag("\x2c")(input)?; // Image separator
        let (input, left) = le_u16(input)?;
        let (input, top) = le_u16(input)?;
        let (input, width) = le_u16(input)?;
        let (input, height) = le_u16(input)?;
        let (input, packed_fields) = le_u8().parse(input)?;

        let (input, local_color_table) = Self::parse_global_color_table(input, packed_fields)?;

        let (input, min_code_size) = le_u8().parse(input)?;

        // Parse image data blocks
        let mut data: Vec<u8> = Vec::new();
        let mut current_input = input;

        loop {
            let (input, block_size) = le_u8().parse(current_input)?;
            if block_size == 0 {
                current_input = input;
                break;
            }

            let (input, block_data) = take(block_size)(input)?;
            data.extend_from_slice(block_data);
            current_input = input;
        }

        Ok((current_input, GifFrame {
            left,
            top,
            width,
            height,
            packed_fields,
            local_color_table,
            min_code_size,
            data,
        }))
    }

    pub fn parse_gif(&mut self, input: &[u8]) -> Result<()> {
        let (input, _) = Self::parse_header(input)
            .map_err(|_| Error::InvalidData)?;

        let (input, (width, height, packed_fields, background_color_index, pixel_aspect_ratio)) =
            Self::parse_logical_screen_descriptor(input)
                .map_err(|_| Error::InvalidData)?;

        self.screen_width = width;
        self.screen_height = height;
        self.packed_fields = packed_fields;

        self.background_color_index = background_color_index;
        self.pixel_aspect_ratio = pixel_aspect_ratio;

        let (mut input, global_color_table) = Self::parse_global_color_table(input, packed_fields)
            .map_err(|_| Error::InvalidData)?;
        self.global_color_table = global_color_table;

        // Parse frames
        while !input.is_empty() {
            match Self::parse_image_descriptor(input) {
                Ok((remaining_input, frame)) => {
                    self.frames.push(frame);
                    input = remaining_input;
                }
                Err(_) => break,
            }
        }

        Ok(())
    }
}

pub struct GifFrame {
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
    pub packed_fields: u8,
    pub local_color_table: Vec<u8>,
    pub min_code_size: u8,
    pub data: Vec<u8>,
}

impl GifFrame {
    pub fn new() -> Self {
        Self {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
            packed_fields: 0,
            local_color_table: Vec::new(),
            min_code_size: 0,
            data: Vec::new(),
        }
    }
}

impl Demuxer for GifDemuxer {
    fn read_headers(&mut self, buf: &mut dyn Buffered, info: &mut GlobalInfo) -> Result<SeekFrom> {
        let data = buf.data();
        self.parse_gif(data)?;

        // Set up stream parameters
        let stream = Stream {
            id: 0,
            index: 0,
            params: CodecParams {
                kind: Some(MediaKind::Video(
                    VideoInfo {
                        width: self.screen_width as usize,
                        height: self.screen_height as usize,
                        format: None,
                    }
                )),
                codec_id: Some("gif".to_string()),
                extradata: Some({
                    let mut extradata = Vec::new();
                    extradata.extend_from_slice(&self.screen_width.to_le_bytes());
                    extradata.extend_from_slice(&self.screen_height.to_le_bytes());
                    extradata.extend_from_slice(&self.packed_fields.to_le_bytes());
                    extradata.extend_from_slice(&self.background_color_index.to_le_bytes());
                    extradata.extend_from_slice(&self.pixel_aspect_ratio.to_le_bytes());
                    extradata.extend_from_slice(&self.global_color_table);
                    extradata
                }),
                bit_rate: 0,
                convergence_window: 0,
                delay: 0
            },
            start: Some(0),
            timebase: Rational64::new(1, 100),
            duration: Some(self.frames.len() as u64),
            user_private: None,
        };

        info.add_stream(stream);

        Ok(SeekFrom::Start(0))
    }

    fn read_event(&mut self, buf: &mut dyn Buffered) -> Result<(SeekFrom, Event)> {
        todo!()
    }
}