pub struct Writer {
    pub data: Vec<u8>,
}

impl Default for Writer {
    fn default() -> Self {
        Writer::new()
    }
}

impl Writer {
    pub fn new() -> Self {
        Writer { data: Vec::new() }
    }

    pub fn u8(&mut self, v: u8) {
        self.data.push(v);
    }

    pub fn u16(&mut self, v: u16) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    pub fn bool(&mut self, v: bool) {
        self.data.push(v as u8);
    }

    pub fn bytes(&mut self, v: &[u8]) {
        self.data.extend_from_slice(v);
    }
}

pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    pub ok: bool,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader {
            data,
            pos: 0,
            ok: true,
        }
    }

    fn take(&mut self, n: usize) -> &'a [u8] {
        if self.pos + n > self.data.len() {
            self.ok = false;
            return &[];
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        s
    }

    pub fn u8(&mut self) -> u8 {
        let s = self.take(1);
        if s.is_empty() {
            0
        } else {
            s[0]
        }
    }

    pub fn u16(&mut self) -> u16 {
        let s = self.take(2);
        if s.len() < 2 {
            0
        } else {
            u16::from_le_bytes([s[0], s[1]])
        }
    }

    pub fn u32(&mut self) -> u32 {
        let s = self.take(4);
        if s.len() < 4 {
            0
        } else {
            u32::from_le_bytes([s[0], s[1], s[2], s[3]])
        }
    }

    pub fn u64(&mut self) -> u64 {
        let s = self.take(8);
        if s.len() < 8 {
            0
        } else {
            u64::from_le_bytes(s.try_into().unwrap())
        }
    }

    pub fn bool(&mut self) -> bool {
        self.u8() != 0
    }

    pub fn bytes(&mut self, out: &mut [u8]) {
        let s = self.take(out.len());
        if s.len() == out.len() {
            out.copy_from_slice(s);
        }
    }

    pub fn done(&self) -> bool {
        self.ok && self.pos == self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_values() {
        for v in [0u16, 1, 0x1234, 0x8000, u16::MAX] {
            let mut w = Writer::new();
            w.u16(v);
            let mut r = Reader::new(&w.data);
            assert_eq!(r.u16(), v);
            assert!(r.done());
        }
        for v in [0u32, 1, 0xDEADBEEF, u32::MAX] {
            let mut w = Writer::new();
            w.u32(v);
            let mut r = Reader::new(&w.data);
            assert_eq!(r.u32(), v);
            assert!(r.done());
        }
    }

    #[test]
    fn underrun_sets_not_ok() {
        let mut r = Reader::new(&[1]);
        r.u16();
        assert!(!r.ok);
    }

    #[test]
    fn mixed_sequence_roundtrip() {
        let mut w = Writer::new();
        w.u8(7);
        w.bool(true);
        w.u64(0x0102030405060708);
        w.bytes(&[9, 8, 7]);
        let mut r = Reader::new(&w.data);
        assert_eq!(r.u8(), 7);
        assert!(r.bool());
        assert_eq!(r.u64(), 0x0102030405060708);
        let mut b = [0u8; 3];
        r.bytes(&mut b);
        assert_eq!(b, [9, 8, 7]);
        assert!(r.done());
    }
}
