use crate::state::{Reader, Writer};

pub struct Controller {
    buttons: u8,
    shift: u8,
    strobe: bool,
}

impl Default for Controller {
    fn default() -> Self {
        Controller::new()
    }
}

impl Controller {
    pub fn new() -> Self {
        Controller {
            buttons: 0,
            shift: 0,
            strobe: false,
        }
    }

    pub fn set_buttons(&mut self, bits: u8) {
        self.buttons = bits;
        if self.strobe {
            self.shift = self.buttons;
        }
    }

    pub fn write_strobe(&mut self, value: u8) {
        self.strobe = value & 1 != 0;
        if self.strobe {
            self.shift = self.buttons;
        }
    }

    pub fn read(&mut self) -> u8 {
        if self.strobe {
            return self.buttons & 1;
        }
        let bit = self.shift & 1;
        self.shift = self.shift >> 1 | 0x80;
        bit
    }

    pub fn save(&self, w: &mut Writer) {
        w.u8(self.buttons);
        w.u8(self.shift);
        w.bool(self.strobe);
    }

    pub fn load(&mut self, r: &mut Reader) {
        self.buttons = r.u8();
        self.shift = r.u8();
        self.strobe = r.bool();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_all_button_patterns_in_order() {
        for pattern in 0..=255u8 {
            let mut c = Controller::new();
            c.set_buttons(pattern);
            c.write_strobe(1);
            c.write_strobe(0);
            for i in 0..8 {
                assert_eq!(c.read(), pattern >> i & 1);
            }
            assert_eq!(c.read(), 1);
            assert_eq!(c.read(), 1);
        }
    }

    #[test]
    fn strobe_high_always_returns_first_button() {
        for pattern in 0..=255u8 {
            let mut c = Controller::new();
            c.set_buttons(pattern);
            c.write_strobe(1);
            for _ in 0..4 {
                assert_eq!(c.read(), pattern & 1);
            }
        }
    }
}
