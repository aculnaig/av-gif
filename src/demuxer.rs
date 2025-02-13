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
    pub comments: Vec<CommentExtension>,
    pub plain_texts: Vec<PlainTextExtension>,
    pub applications: Vec<ApplicationExtension>,
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
            comments: Vec::new(),
            plain_texts: Vec::new(),
            applications: Vec::new(),
        }
    }

    pub fn get_comments(&self) -> &[CommentExtension] {
        &self.comments
    }

    pub fn get_plain_texts(&self) -> &[PlainTextExtension] {
        &self.plain_texts
    }

    pub fn get_applications(&self) -> &[ApplicationExtension] {
        &self.applications
    }

    // Helper method to find NETSCAPE 2.0 application extension for animation loop count 
    pub fn get_loop_count(&self) -> Option<u16> {
        self.applications.iter()
            .find(|app| app.identifier == "NETSCAPE" && app.auth_code == *b"2.0")
            .and_then(|app| {
                if app.data.len() >= 3 && app.data[0] == 1 {
                    Some(u16::from_le_bytes([app.data[1], app.data[2]]))
                } else {
                    None
                }
            })
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

    // Extensions parsing
    // Comment extension
    pub fn parse_comment_extension(input: &[u8]) -> IResult<&[u8], CommentExtension> {
        let mut text = Vec::new();
        let mut current_input = input;

        loop {
            let (remaining, block_size) = le_u8().parse(current_input)?;
            if block_size == 0 {
                current_input = remaining;
                break;
            }
            let (remaining, block_data) = take(block_size as usize)(remaining)?;
            text.extend_from_slice(block_data);
            current_input = remaining;
        }

        // Convert to string, replacing invalid UTF-8 sequences
        let text_str = String::from_utf8_lossy(&text).into_owned();

        Ok((current_input, CommentExtension { text: text_str }))
    }

    // Plaintext extension
    pub fn parse_plain_text_extension(input: &[u8]) -> IResult<&[u8], PlainTextExtension> {
        let (input, block_size) = le_u8().parse(input)?; // Should be 12
        if block_size != 12 {
            return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify)));
        }

        let (input, text_grid_left) = le_u16(input)?;
        let (input, text_grid_top) = le_u16(input)?;
        let (input, text_grid_width) = le_u16(input)?;
        let (input, text_grid_height) = le_u16(input)?;
        let (input, char_cell_width) = le_u8().parse(input)?;
        let (input, char_cell_height) = le_u8().parse(input)?;
        let (input, text_foreground_color_index) = le_u8().parse(input)?;
        let (input, text_background_color_index) = le_u8().parse(input)?;

        // Parse text sub-blocks
        let mut text = Vec::new();
        let mut current_input = input;

        loop {
            let (remaining, block_size) = le_u8().parse(current_input)?;
            if block_size == 0 {
                current_input = remaining;
                break;
            }

            let (remaining, block_data) = take(block_size as usize)(remaining)?;
            text.extend_from_slice(block_data);
            current_input = remaining;
        }

        let text_str = String::from_utf8_lossy(&text).into_owned();

        Ok((current_input, PlainTextExtension {
            text_grid_left,
            text_grid_top,
            text_grid_width,
            text_grid_height,
            char_cell_width,
            char_cell_height,
            text_foreground_color_index,
            text_background_color_index,
            text: text_str
        }))
    }

    // Application extension
    pub fn parse_application_extension(input: &[u8]) -> IResult<&[u8], ApplicationExtension> {
        let (input, block_size) = le_u8().parse(input)?; // Should be 11
        if block_size != 11 {
            return Err(nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify)));
        }

        let (input, application_identifier) = take(8usize)(input)?;
        let (input, auth_code) = take(3usize)(input)?;

        // Parse application data sub-blocks
        let mut data = Vec::new();
        let mut current_input = input;

        loop {
            let (remaining, block_size) = le_u8().parse(current_input)?;
            if block_size == 0 {
                current_input = remaining;
                break;
            }

            let (remaining, block_data) = take(block_size as usize)(remaining)?;
            data.extend_from_slice(block_data);
            current_input = remaining;
        }

        let identifier = String::from_utf8_lossy(application_identifier).into_owned();
        let mut auth_code_array = [0u8; 3];
        auth_code_array.copy_from_slice(auth_code);

        Ok((current_input, ApplicationExtension {
            identifier,
            auth_code: auth_code_array,
            data,
        }))
    }

    // GCE parsing
    pub fn parse_graphics_control_extension(input: &[u8]) -> IResult<&[u8], GraphicsControlExtension> {
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

    pub fn parse_block(input: &[u8]) -> IResult<&[u8], Option<(Option<Extension>, Option<GifFrame>)>> {
        let (mut current_input, block_type) = le_u8().parse(input)?;

        match block_type {
            0x21 => { // Extension Introducer
                let (remaining, label) = le_u8().parse(current_input)?;
                current_input = remaining;

                match label {
                    0xf9 => { // Graphics Control Extension
                        let (remaining, gce) = Self::parse_graphics_control_extension(current_input)?;
                        current_input = remaining;
                        Ok((current_input, Some((Some(Extension::GraphicsControl(gce)), None))))
                    }

                    0xfe => { // Comment Extension
                        let (remaining, comment) = Self::parse_comment_extension(current_input)?;
                        current_input = remaining;
                        Ok((current_input, Some((Some(Extension::Comment(comment)), None))))
                    }

                    0x01 => { // Plain Text Extension
                        let (remaining, plain_text) = Self::parse_plain_text_extension(current_input)?;
                        current_input = remaining;
                        Ok((current_input, Some((Some(Extension::PlainText(plain_text)), None))))
                    }

                    0xff => { // Application Extension
                        let (remaining, app) = Self::parse_application_extension(current_input)?;
                        current_input = remaining;
                        Ok((current_input, Some((Some(Extension::Application(app)), None))))
                    }

                    // Unknown Extension
                    _ => Ok((current_input, None))
                }
            }

            0x2c => { // Image Descriptor
                let (input, frame) = Self::parse_image_descriptor(input)?;
                Ok((input, Some((None, Some(frame)))))
            }

            0x3b => { // Trailer
                Ok((current_input, None))
            }

            // Unknown block
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
                Ok((remaining, Some((extension, frame)))) => {
                    input = remaining;

                    if let Some(ext) = extension {
                        match ext {
                            Extension::GraphicsControl(gce) => pending_gce = Some(gce),
                            Extension::Comment(comment) => self.comments.push(comment),
                            Extension::PlainText(text) => self.plain_texts.push(text),
                            Extension::Application(app) => self.applications.push(app),
                        }
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
                    break;
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum Extension {
    GraphicsControl(GraphicsControlExtension),
    Comment(CommentExtension),
    PlainText(PlainTextExtension),
    Application(ApplicationExtension),
}

#[derive(Debug, Clone)]
pub struct GraphicsControlExtension {
    pub disposal_method: u8,
    pub user_input_flag: bool,
    pub transparent_color_flag: bool,
    pub delay_time: u16,
    pub transparent_color_index: u8,
}

#[derive(Debug, Clone)]
pub struct CommentExtension {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct PlainTextExtension {
    pub text_grid_left: u16,
    pub text_grid_top: u16,
    pub text_grid_width: u16,
    pub text_grid_height: u16,
    pub char_cell_width: u8,
    pub char_cell_height: u8,
    pub text_foreground_color_index: u8,
    pub text_background_color_index: u8,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ApplicationExtension {
    pub identifier: String,
    pub auth_code: [u8; 3],
    pub data: Vec<u8>,
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

mod tests {
    use av_format::demuxer;

    use crate::demuxer::GifDemuxer;

    #[test]
    fn test_parse_gif87a() {
        let buf = include_bytes!("../assets/87a.gif");

        let mut demuxer = GifDemuxer::new();
        demuxer.parse_gif(buf).unwrap();

        assert_eq!(demuxer.screen_width, 100);
        assert_eq!(demuxer.screen_height, 100);
    }

    #[test]
    fn test_parse_gif89a() {
        let buf = include_bytes!("../assets/89a.gif");

        let mut demuxer = GifDemuxer::new();
        demuxer.parse_gif(buf).unwrap();

        assert_eq!(demuxer.screen_width, 100);
        assert_eq!(demuxer.screen_height, 100);
    }

    #[test]
    fn test_parse_gif89a_multi_frame() {
        let buf = include_bytes!("../assets/multi_frame.gif");

        let mut demuxer = GifDemuxer::new();
        demuxer.parse_gif(buf).unwrap();

        assert_eq!(demuxer.screen_width, 100);
        assert_eq!(demuxer.screen_height, 100);
        assert_eq!(demuxer.frames.len(), 2);
    }

    #[test]
    fn test_parse_gif_infinite_loop() {
        let buf = include_bytes!("../assets/infinite_loop.gif");

        let mut demuxer = GifDemuxer::new();
        demuxer.parse_gif(buf).unwrap();

        assert_eq!(demuxer.screen_width, 100);
        assert_eq!(demuxer.screen_height, 100);

        // Check for NETSCAPE 2.0 extension
        let loop_count = demuxer.get_loop_count();
        assert!(loop_count.is_some());
    }

    #[test]
    fn test_parse_gif_transparency() {
        let buf = include_bytes!("../assets/transparency.gif");

        let mut demuxer = GifDemuxer::new();
        demuxer.parse_gif(buf).unwrap();

        assert_eq!(demuxer.screen_width, 100);
        assert_eq!(demuxer.screen_height, 100);

        // Check if any frame has transparency
        let has_transparency = demuxer.frames.iter().any(|frame| {
            frame.gce.is_some() && frame.gce.as_ref().unwrap().transparent_color_flag
        });

        assert!(has_transparency);
    }

    #[test]
    fn test_parse_gif_comments() {
        let buf = include_bytes!("../assets/comments.gif");

        let mut demuxer = GifDemuxer::new();
        demuxer.parse_gif(buf).unwrap();

        assert_eq!(demuxer.screen_width, 100);
        assert_eq!(demuxer.screen_height, 100);

        // Check if any frame has comments
        let has_comments = demuxer.comments.len() > 0;
        let comment = demuxer.comments.first().unwrap();

        assert!(has_comments);
        assert_eq!(comment.text, "This is a comment");
    }

    #[test]
    fn test_parse_gif_plain_text() {
        let buf = include_bytes!("../assets/plain_text.gif");

        let mut demuxer = GifDemuxer::new();
        demuxer.parse_gif(buf).unwrap();

        assert_eq!(demuxer.screen_width, 100);
        assert_eq!(demuxer.screen_height, 100);

        // Check if any frame has plain text
        let has_plain_text = demuxer.plain_texts.len() > 0;
        let plain_text = demuxer.plain_texts.first().unwrap();

        assert!(has_plain_text);
        assert_eq!(plain_text.text, "The filename is:", "plain_text.gif");
    }
}
