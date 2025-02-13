use std::io::SeekFrom;

use av_format::{buffer::Buffered, common::GlobalInfo, demuxer::{Demuxer, Event}};
use av_format::error::Result;
use nom::IResult;

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

    pub fn parse_gif_header(input: &[u8]) -> IResult<&[u8], ()> {
        todo!()
    }

    pub fn parse_logical_screen_descriptor(input: &[u8]) -> IResult<&[u8], (u16, u16, u8, u8, u8)> {
        todo!()
    }

    pub fn parse_global_color_table(input: &[u8], packed_fields: u8) -> IResult<&[u8], Vec<u8>> {
        todo!()
    }

    pub fn parse_image_descriptor(input: &[u8]) -> IResult<&[u8], GifFrame> {
        todo!()
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

impl Demuxer for GifDemuxer {
    fn read_headers(&mut self, buf: &mut dyn Buffered, info: &mut GlobalInfo) -> Result<SeekFrom> {
        todo!()
    }

    fn read_event(&mut self, buf: &mut dyn Buffered) -> Result<(SeekFrom, Event)> {
        todo!()
    }
}