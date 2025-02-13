use std::io::SeekFrom;

use av_data::{packet::Packet, params::{CodecParams, MediaKind, VideoInfo}, rational::Rational64, timeinfo::TimeInfo};
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
    pub current_frame: u64,
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
            current_frame: 0,
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

    // GCE parsing
    pub fn parse_graphics_control_extension(input: &[u8]) -> IResult<&[u8], GraphicsControlExtension> {
        let (input, extension_indtroducer) = le_u8().parse(input)?;
        if extension_indtroducer != 0x21 {
            return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)))
        }
        let (input, extension_label) = le_u8().parse(input)?;
        if extension_label != 0xf9 {
            return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Tag)));
        }
        let (input, block_size) = le_u8().parse(input)?;

        if block_size != 4 {
            return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify)));
        }

        let (input, packed_fields) = le_u8().parse(input)?;
        let (input, delay_time) = le_u16(input)?;
        let (input, transparent_color_index) = le_u8().parse(input)?;
        let (input, _) = tag("\x00")(input)?; // Block terminator

        Ok((input, GraphicsControlExtension {
            disposal_method: (packed_fields >> 2) & 0x07,
            user_input_flag: (packed_fields & 0x02) != 0,
            transparent_color_flag: (packed_fields & 0x01) != 0,
            delay_time,
            transparent_color_index,
        }))
    }

    pub fn parse_block(input: &[u8]) -> IResult<&[u8], Option<(Option<GraphicsControlExtension>, Option<GifFrame>)>> {
        let (mut current_input, block_type) = le_u8().parse(input)?;

        match block_type {
            0x21 => { // Extension Introducer
                let (remaining, label) = le_u8().parse(input)?;
                current_input = remaining;

                match label { // Graphics Control Extension
                    0xF9 => {
                        let (input, gce) = Self::parse_graphics_control_extension(input)?;
                        current_input = input;

                        // After GCE, there might be an image descriptor
                        match le_u8::<&[u8], nom::error::Error<&[u8]>>().parse(current_input) {
                            Ok((input, 0x2c)) => {
                                let (input, mut frame) = Self::parse_image_descriptor(input)?;
                                frame.gce = Some(gce);
                                Ok((input, Some((None, Some(frame)))))
                            }
                            _ => Ok((current_input, Some((Some(gce), None))))
                        }
                    }

                    // Handle other extensions (like Comment Extensions, Plain Text Extension, etc...)
                    0xfe | 0x01 | 0xff => {
                        // Skip other extensions
                        let mut input = current_input;
                        loop {
                            let (new_input, block_size) = le_u8().parse(input)?;
                            if block_size == 0 {
                                return Ok((new_input, None));
                            }
                            let (new_input, _) = take(block_size as usize)(new_input)?;
                            input = new_input;
                        }
                    }
                    _ => Ok((current_input, None))
                }
            }

            0x2c => {
                // Image Descriptor
                let (input, frame) = Self::parse_image_descriptor(current_input)?;
                Ok((input, Some((None, Some(frame)))))
            }

            0x3b => {
                // Trailer
                Ok((current_input, Some((None, None))))
            }

            _ => Ok((current_input, None))
        }
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
            gce: None,
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

        // Parse blocks until we reach the end
        let mut pending_gce: Option<GraphicsControlExtension> = None;

        // Parse frames
        while !input.is_empty() {
            match Self::parse_block(input) {
                Ok((remaining, Some((gce, frame)))) => {
                    input = remaining;

                    if let Some(gce) = gce {
                        pending_gce = Some(gce);
                    }

                    if let Some(mut frame) = frame {
                        if pending_gce.is_some() {
                            frame.gce = pending_gce.take();
                        }
                        self.frames.push(frame);
                    }
                }

                Ok((remaining, None)) => {
                    input = remaining;
                }

                Err(_) => {
                    return Err(Error::InvalidData);
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct GraphicsControlExtension {
    pub disposal_method: u8,
    pub user_input_flag: bool,
    pub transparent_color_flag: bool,
    pub delay_time: u16,
    pub transparent_color_index: u8,
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
    pub gce: Option<GraphicsControlExtension>,
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
            gce: None,
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
        // Check if we have processed all frames
        if self.current_frame >= self.frames.len() as u64 {
            return Ok((SeekFrom::Current(0), Event::Eof));
        }

        // Get the current frame
        let frame = &self.frames[self.current_frame as usize];

        // Create packet data
        let mut packet_data = Vec::new();

        // Add frame header information
        packet_data.extend_from_slice(&frame.left.to_le_bytes());
        packet_data.extend_from_slice(&frame.top.to_le_bytes());
        packet_data.extend_from_slice(&frame.width.to_le_bytes());
        packet_data.push(frame.packed_fields);

        // Add GCE information if present
        if let Some(gce) = &frame.gce {
            let mut gce_packed = 0u8;
            gce_packed |= (gce.disposal_method & 0x07) << 2;
            gce_packed |= (gce.user_input_flag as u8) << 1;
            gce_packed |= gce.transparent_color_flag as u8;

            packet_data.push(gce_packed);
            packet_data.extend_from_slice(&gce.delay_time.to_le_bytes());
            packet_data.push(gce.transparent_color_index);
        } else {
            packet_data.push(0x00); // No GCE
        }

        // Add local color table if present
        if !frame.local_color_table.is_empty() {
            packet_data.extend_from_slice(&frame.local_color_table);
        }

        // Add LZW minimum code size
        packet_data.push(frame.min_code_size);

        // Add image data
        packet_data.extend_from_slice(&frame.data);

        // Create the packet
        let packet = Packet {
            stream_index: 0,
            data: packet_data,
            pos: None,
            t: TimeInfo {
                pts: Some(self.current_frame as i64),
                dts: Some(self.current_frame as i64),
                duration: Some(if let Some(gce) = &frame.gce { gce.delay_time as u64 } else { 1 }),
                timebase: Some(Rational64::new(1, 100)),
                user_private: None,
            },
            is_key: true,
            is_corrupted: false,
        };

        // Increment the frame counter
        self.current_frame += 1;

        Ok((SeekFrom::Current(0), Event::NewPacket(packet)))
    }
}