use std::collections::HashMap;

pub struct LzwEncoder {
    code_size: u8,                     // Number of bits per code
    clear_code: u16,                   // 256
    end_of_stream_code: u16,           // 257
    next_code: u16,                    // Next available dictionary index
    dictionary: HashMap<Vec<u8>, u16>, // LZW dictionary
    current_sequence: Vec<u8>,         // Current sequence being encoded
    output: Vec<u8>,                   // Encoded data
}

impl LzwEncoder {
    pub fn new() -> Self {
        let mut dictionary = HashMap::new();

        // Initialize dictionary with single-byte values
        for i in 0u16..=255 {
            dictionary.insert(vec![i as u8], i);
        }

        Self {
            code_size: 9, // Starts at 9 bits (8-bit color + control codes)
            clear_code: 256,
            end_of_stream_code: 257,
            next_code: 258,
            dictionary,
            current_sequence: Vec::new(),
            output: Vec::new(),
        }
    }

    pub fn encode_chunk(&mut self, chunk: &[u8]) {
        for &pixel in chunk {
            let mut extended_sequence = self.current_sequence.clone();
            extended_sequence.push(pixel);

            if self.dictionary.contains_key(&extended_sequence) {
                self.current_sequence = extended_sequence;
            } else {
                let code = self.dictionary[&self.current_sequence];
                self.write_code(code);

                if self.next_code < (1 << 12) {
                    self.dictionary.insert(extended_sequence, self.next_code);
                    self.next_code += 1;
                    if self.next_code == (1 << self.code_size) {
                        // Increase code size when necessary
                        self.code_size += 1;
                    }
                } else {
                    // Reset dictionary when full
                    self.write_code(self.clear_code);
                    self.dictionary.clear();
                    for i in 0u16..=255 {
                        self.dictionary.insert(vec![i as u8], i);
                    }
                    self.next_code = 258;
                    self.code_size = 9;
                }

                self.current_sequence = vec![pixel];
            }
        }
    }

    pub fn finalize(&mut self) {
        if !self.current_sequence.is_empty() {
            let code = self.dictionary[&self.current_sequence];
            self.write_code(code);
        }
        self.write_code(self.end_of_stream_code);
    }

    fn write_code(&mut self, code: u16) {
        let num_bits = self.code_size as usize;
        let bits = format!("{:0width$b}", code, width = num_bits)
            .chars()
            .rev()
            .collect::<Vec<char>>();

        self.output.extend(bits.iter().map(|&b| b as u8));
    }

    pub fn get_encoded_data(&self) -> &[u8] {
        &self.output
    }
}
