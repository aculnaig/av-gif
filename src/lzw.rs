use std::collections::HashMap;

pub struct LzwEncoder {
    code_size: u8,                     // Number of bits per code
    clear_code: u16,                   // 256
    end_of_stream_code: u16,           // 257
    next_code: u16,                    // Next available dictionary index
    dictionary: HashMap<Vec<u8>, u16>, // LZW dictionary
    current_sequence: Vec<u8>,         // Current sequence being encoded
    output: Vec<u8>,                   // Encoded data
    bit_buffer: u32,                   // Buffer for packing bits
    bit_count: u32,                     // Number of bits in the current bit buffer
}

impl LzwEncoder {
    pub fn new(code_size: u8) -> Self {
        let mut dictionary = HashMap::new();

        // Initialize dictionary with single-byte values
        for i in 0u16..=255 {
            dictionary.insert(vec![i as u8], i);
        }

        Self {
            code_size: code_size,
            clear_code: 256,
            end_of_stream_code: 257,
            next_code: 258,
            dictionary,
            current_sequence: Vec::new(),
            output: Vec::new(),
            bit_buffer: 0,
            bit_count: 0,
        }
    }

    pub fn encode_chunk(&mut self, chunk: &[u8]) {
        // Write clear code at the start of the image data
        if self.output.is_empty() {
            self.write_code(self.clear_code);
        }

        for &pixel in chunk {
            let mut extended_sequence = self.current_sequence.clone();
            extended_sequence.push(pixel);

            if self.dictionary.contains_key(&extended_sequence) {
                self.current_sequence = extended_sequence;
            } else {
                let code = self.dictionary[&self.current_sequence];
                self.write_code(code);

                if self.next_code < 4096 {
                    self.dictionary.insert(extended_sequence, self.next_code);
                    self.next_code += 1;

                    // Increase code size before writing the next code
                    if self.next_code == (1 << self.code_size) - 1 && self.code_size < 12 {
                        self.code_size += 1;
                    }

                    let code = self.dictionary[&self.current_sequence];
                    self.write_code(code);
                } else {
                    // Reset dictionary when full
                    self.write_code(self.clear_code);
                    self.reset_dictionary();
                }

                self.current_sequence.clear();
                self.current_sequence.push(pixel);
            }
        }
    }

    pub fn finalize(&mut self) {
        if !self.current_sequence.is_empty() {
            let code = self.dictionary[&self.current_sequence];
            self.write_code(code);
        }
        self.write_code(self.end_of_stream_code);

        // Flush remaining bits to output
        while self.bit_count > 0 {
            self.output.push(self.bit_buffer as u8);
            self.bit_buffer >>= 8;
            self.bit_count = self.bit_count.saturating_sub(8);
        }
    }

    fn write_code(&mut self, code: u16) {
        self.bit_buffer |= (code as u32) << self.bit_count;
        self.bit_count += self.code_size as u32;

        // Flush bits if we have 8 or more
        while self.bit_count >= 8 {
            self.flush_bits();
        }
    }

    fn flush_bits(&mut self) {
        self.output.push(self.bit_buffer as u8);
        self.bit_buffer >>= 8;
        self.bit_count -= 8;
    }

    fn reset_dictionary(&mut self) {
        self.dictionary.clear();

        for i in 0u16..=255 {
            self.dictionary.insert(vec![i as u8], i);
        }

        self.next_code = 258;
        self.code_size = 9;
        self.current_sequence.clear();
    }

    pub fn reset(&mut self) {
        self.reset_dictionary();

        self.bit_buffer = 0;
        self.bit_count = 0;
    }

    pub fn get_encoded_data(&self) -> &[u8] {
        &self.output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lzw_encoder() {
        let mut encoder = LzwEncoder::new(2);
        let chunk = b"ABABABABABABABABA";

        encoder.encode_chunk(Vec::from(chunk).as_ref());
        encoder.finalize();

        let encoded_data = encoder.get_encoded_data();

        // Assert that the encoded data is not empty
        assert!(!encoded_data.is_empty());

        // Assert that the encoded data is not the same as the input data
        assert_ne!(encoded_data, chunk);
    }
}
