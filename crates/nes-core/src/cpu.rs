use crate::bus::Bus;
use crate::state::{Reader, Writer};

const C: u8 = 0x01;
const Z: u8 = 0x02;
const I: u8 = 0x04;
const D: u8 = 0x08;
const B: u8 = 0x10;
const U: u8 = 0x20;
const V: u8 = 0x40;
const N: u8 = 0x80;

pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub p: u8,
    pub nmi_pending: bool,
    pub irq_line: bool,
}

impl Default for Cpu {
    fn default() -> Self {
        Cpu::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0,
            p: 0x24,
            nmi_pending: false,
            irq_line: false,
        }
    }

    pub fn reset(&mut self, bus: &mut impl Bus) {
        self.sp = 0xFD;
        self.p |= I;
        self.pc = self.read16(bus, 0xFFFC);
    }

    pub fn save(&self, w: &mut Writer) {
        w.u8(self.a);
        w.u8(self.x);
        w.u8(self.y);
        w.u8(self.sp);
        w.u16(self.pc);
        w.u8(self.p);
        w.bool(self.nmi_pending);
        w.bool(self.irq_line);
    }

    pub fn load(&mut self, r: &mut Reader) {
        self.a = r.u8();
        self.x = r.u8();
        self.y = r.u8();
        self.sp = r.u8();
        self.pc = r.u16();
        self.p = r.u8();
        self.nmi_pending = r.bool();
        self.irq_line = r.bool();
    }

    pub fn step(&mut self, bus: &mut impl Bus) -> u32 {
        if self.nmi_pending {
            self.nmi_pending = false;
            self.interrupt(bus, 0xFFFA);
            return 7;
        }
        if self.irq_line && self.p & I == 0 {
            self.interrupt(bus, 0xFFFE);
            return 7;
        }
        let op = self.fetch(bus);
        macro_rules! rd {
            ($m:ident, $c:expr, $f:ident) => {{
                let (addr, crossed) = self.$m(bus);
                let v = bus.read(addr);
                self.$f(v);
                $c + crossed as u32
            }};
        }
        macro_rules! st {
            ($m:ident, $c:expr, $v:expr) => {{
                let (addr, _) = self.$m(bus);
                let v = $v;
                bus.write(addr, v);
                $c
            }};
        }
        macro_rules! rmw {
            ($m:ident, $c:expr, $f:ident) => {{
                let (addr, _) = self.$m(bus);
                let v = bus.read(addr);
                bus.write(addr, v);
                let r = self.$f(v);
                bus.write(addr, r);
                $c
            }};
        }
        macro_rules! acc {
            ($f:ident) => {{
                bus.read(self.pc);
                let r = self.$f(self.a);
                self.a = r;
                2
            }};
        }
        macro_rules! imp {
            ($b:expr) => {{
                bus.read(self.pc);
                $b;
                2
            }};
        }
        match op {
            0x69 => rd!(imm, 2, adc),
            0x65 => rd!(zp, 3, adc),
            0x75 => rd!(zpx, 4, adc),
            0x6D => rd!(abs, 4, adc),
            0x7D => rd!(abx, 4, adc),
            0x79 => rd!(aby, 4, adc),
            0x61 => rd!(izx, 6, adc),
            0x71 => rd!(izy, 5, adc),
            0xE9 | 0xEB => rd!(imm, 2, sbc),
            0xE5 => rd!(zp, 3, sbc),
            0xF5 => rd!(zpx, 4, sbc),
            0xED => rd!(abs, 4, sbc),
            0xFD => rd!(abx, 4, sbc),
            0xF9 => rd!(aby, 4, sbc),
            0xE1 => rd!(izx, 6, sbc),
            0xF1 => rd!(izy, 5, sbc),
            0x29 => rd!(imm, 2, and),
            0x25 => rd!(zp, 3, and),
            0x35 => rd!(zpx, 4, and),
            0x2D => rd!(abs, 4, and),
            0x3D => rd!(abx, 4, and),
            0x39 => rd!(aby, 4, and),
            0x21 => rd!(izx, 6, and),
            0x31 => rd!(izy, 5, and),
            0x09 => rd!(imm, 2, ora),
            0x05 => rd!(zp, 3, ora),
            0x15 => rd!(zpx, 4, ora),
            0x0D => rd!(abs, 4, ora),
            0x1D => rd!(abx, 4, ora),
            0x19 => rd!(aby, 4, ora),
            0x01 => rd!(izx, 6, ora),
            0x11 => rd!(izy, 5, ora),
            0x49 => rd!(imm, 2, eor),
            0x45 => rd!(zp, 3, eor),
            0x55 => rd!(zpx, 4, eor),
            0x4D => rd!(abs, 4, eor),
            0x5D => rd!(abx, 4, eor),
            0x59 => rd!(aby, 4, eor),
            0x41 => rd!(izx, 6, eor),
            0x51 => rd!(izy, 5, eor),
            0xC9 => rd!(imm, 2, cmp),
            0xC5 => rd!(zp, 3, cmp),
            0xD5 => rd!(zpx, 4, cmp),
            0xCD => rd!(abs, 4, cmp),
            0xDD => rd!(abx, 4, cmp),
            0xD9 => rd!(aby, 4, cmp),
            0xC1 => rd!(izx, 6, cmp),
            0xD1 => rd!(izy, 5, cmp),
            0xE0 => rd!(imm, 2, cpx),
            0xE4 => rd!(zp, 3, cpx),
            0xEC => rd!(abs, 4, cpx),
            0xC0 => rd!(imm, 2, cpy),
            0xC4 => rd!(zp, 3, cpy),
            0xCC => rd!(abs, 4, cpy),
            0x24 => rd!(zp, 3, bit),
            0x2C => rd!(abs, 4, bit),
            0xA9 => rd!(imm, 2, lda),
            0xA5 => rd!(zp, 3, lda),
            0xB5 => rd!(zpx, 4, lda),
            0xAD => rd!(abs, 4, lda),
            0xBD => rd!(abx, 4, lda),
            0xB9 => rd!(aby, 4, lda),
            0xA1 => rd!(izx, 6, lda),
            0xB1 => rd!(izy, 5, lda),
            0xA2 => rd!(imm, 2, ldx),
            0xA6 => rd!(zp, 3, ldx),
            0xB6 => rd!(zpy, 4, ldx),
            0xAE => rd!(abs, 4, ldx),
            0xBE => rd!(aby, 4, ldx),
            0xA0 => rd!(imm, 2, ldy),
            0xA4 => rd!(zp, 3, ldy),
            0xB4 => rd!(zpx, 4, ldy),
            0xAC => rd!(abs, 4, ldy),
            0xBC => rd!(abx, 4, ldy),
            0xAB => rd!(imm, 2, lax),
            0xA7 => rd!(zp, 3, lax),
            0xB7 => rd!(zpy, 4, lax),
            0xAF => rd!(abs, 4, lax),
            0xBF => rd!(aby, 4, lax),
            0xA3 => rd!(izx, 6, lax),
            0xB3 => rd!(izy, 5, lax),
            0x85 => st!(zp, 3, self.a),
            0x95 => st!(zpx, 4, self.a),
            0x8D => st!(abs, 4, self.a),
            0x9D => st!(abx_w, 5, self.a),
            0x99 => st!(aby_w, 5, self.a),
            0x81 => st!(izx, 6, self.a),
            0x91 => st!(izy_w, 6, self.a),
            0x86 => st!(zp, 3, self.x),
            0x96 => st!(zpy, 4, self.x),
            0x8E => st!(abs, 4, self.x),
            0x84 => st!(zp, 3, self.y),
            0x94 => st!(zpx, 4, self.y),
            0x8C => st!(abs, 4, self.y),
            0x87 => st!(zp, 3, self.a & self.x),
            0x97 => st!(zpy, 4, self.a & self.x),
            0x8F => st!(abs, 4, self.a & self.x),
            0x83 => st!(izx, 6, self.a & self.x),
            0x0A => acc!(asl),
            0x06 => rmw!(zp, 5, asl),
            0x16 => rmw!(zpx, 6, asl),
            0x0E => rmw!(abs, 6, asl),
            0x1E => rmw!(abx_w, 7, asl),
            0x4A => acc!(lsr),
            0x46 => rmw!(zp, 5, lsr),
            0x56 => rmw!(zpx, 6, lsr),
            0x4E => rmw!(abs, 6, lsr),
            0x5E => rmw!(abx_w, 7, lsr),
            0x2A => acc!(rol),
            0x26 => rmw!(zp, 5, rol),
            0x36 => rmw!(zpx, 6, rol),
            0x2E => rmw!(abs, 6, rol),
            0x3E => rmw!(abx_w, 7, rol),
            0x6A => acc!(ror),
            0x66 => rmw!(zp, 5, ror),
            0x76 => rmw!(zpx, 6, ror),
            0x6E => rmw!(abs, 6, ror),
            0x7E => rmw!(abx_w, 7, ror),
            0xE6 => rmw!(zp, 5, inc),
            0xF6 => rmw!(zpx, 6, inc),
            0xEE => rmw!(abs, 6, inc),
            0xFE => rmw!(abx_w, 7, inc),
            0xC6 => rmw!(zp, 5, dec),
            0xD6 => rmw!(zpx, 6, dec),
            0xCE => rmw!(abs, 6, dec),
            0xDE => rmw!(abx_w, 7, dec),
            0x07 => rmw!(zp, 5, slo),
            0x17 => rmw!(zpx, 6, slo),
            0x0F => rmw!(abs, 6, slo),
            0x1F => rmw!(abx_w, 7, slo),
            0x1B => rmw!(aby_w, 7, slo),
            0x03 => rmw!(izx, 8, slo),
            0x13 => rmw!(izy_w, 8, slo),
            0x27 => rmw!(zp, 5, rla),
            0x37 => rmw!(zpx, 6, rla),
            0x2F => rmw!(abs, 6, rla),
            0x3F => rmw!(abx_w, 7, rla),
            0x3B => rmw!(aby_w, 7, rla),
            0x23 => rmw!(izx, 8, rla),
            0x33 => rmw!(izy_w, 8, rla),
            0x47 => rmw!(zp, 5, sre),
            0x57 => rmw!(zpx, 6, sre),
            0x4F => rmw!(abs, 6, sre),
            0x5F => rmw!(abx_w, 7, sre),
            0x5B => rmw!(aby_w, 7, sre),
            0x43 => rmw!(izx, 8, sre),
            0x53 => rmw!(izy_w, 8, sre),
            0x67 => rmw!(zp, 5, rra),
            0x77 => rmw!(zpx, 6, rra),
            0x6F => rmw!(abs, 6, rra),
            0x7F => rmw!(abx_w, 7, rra),
            0x7B => rmw!(aby_w, 7, rra),
            0x63 => rmw!(izx, 8, rra),
            0x73 => rmw!(izy_w, 8, rra),
            0xC7 => rmw!(zp, 5, dcp),
            0xD7 => rmw!(zpx, 6, dcp),
            0xCF => rmw!(abs, 6, dcp),
            0xDF => rmw!(abx_w, 7, dcp),
            0xDB => rmw!(aby_w, 7, dcp),
            0xC3 => rmw!(izx, 8, dcp),
            0xD3 => rmw!(izy_w, 8, dcp),
            0xE7 => rmw!(zp, 5, isb),
            0xF7 => rmw!(zpx, 6, isb),
            0xEF => rmw!(abs, 6, isb),
            0xFF => rmw!(abx_w, 7, isb),
            0xFB => rmw!(aby_w, 7, isb),
            0xE3 => rmw!(izx, 8, isb),
            0xF3 => rmw!(izy_w, 8, isb),
            0x0B | 0x2B => rd!(imm, 2, anc),
            0x4B => rd!(imm, 2, alr),
            0x6B => rd!(imm, 2, arr),
            0xCB => rd!(imm, 2, axs),
            0x9C => {
                let base = self.fetch16(bus);
                let value = self.y;
                self.sh_store(bus, base, self.x, value);
                5
            }
            0x9E => {
                let base = self.fetch16(bus);
                let value = self.x;
                self.sh_store(bus, base, self.y, value);
                5
            }
            0xAA => imp!({
                self.x = self.a;
                self.set_zn(self.x)
            }),
            0xA8 => imp!({
                self.y = self.a;
                self.set_zn(self.y)
            }),
            0x8A => imp!({
                self.a = self.x;
                self.set_zn(self.a)
            }),
            0x98 => imp!({
                self.a = self.y;
                self.set_zn(self.a)
            }),
            0xBA => imp!({
                self.x = self.sp;
                self.set_zn(self.x)
            }),
            0x9A => imp!(self.sp = self.x),
            0xE8 => imp!(self.x = self.inc(self.x)),
            0xC8 => imp!(self.y = self.inc(self.y)),
            0xCA => imp!(self.x = self.dec(self.x)),
            0x88 => imp!(self.y = self.dec(self.y)),
            0x18 => imp!(self.set_flag(C, false)),
            0x38 => imp!(self.set_flag(C, true)),
            0x58 => imp!(self.set_flag(I, false)),
            0x78 => imp!(self.set_flag(I, true)),
            0xD8 => imp!(self.set_flag(D, false)),
            0xF8 => imp!(self.set_flag(D, true)),
            0xB8 => imp!(self.set_flag(V, false)),
            0x48 => {
                bus.read(self.pc);
                self.push(bus, self.a);
                3
            }
            0x08 => {
                bus.read(self.pc);
                self.push(bus, self.p | B | U);
                3
            }
            0x68 => {
                bus.read(self.pc);
                bus.read(0x0100 | self.sp as u16);
                self.a = self.pull(bus);
                self.set_zn(self.a);
                4
            }
            0x28 => {
                bus.read(self.pc);
                bus.read(0x0100 | self.sp as u16);
                self.p = self.pull(bus) & !B | U;
                4
            }
            0x4C => {
                self.pc = self.fetch16(bus);
                3
            }
            0x6C => {
                let p = self.fetch16(bus);
                let lo = bus.read(p) as u16;
                let hi = bus.read(p & 0xFF00 | (p as u8).wrapping_add(1) as u16) as u16;
                self.pc = hi << 8 | lo;
                5
            }
            0x20 => {
                let lo = self.fetch(bus) as u16;
                bus.read(0x0100 | self.sp as u16);
                self.push16(bus, self.pc);
                let hi = bus.read(self.pc) as u16;
                self.pc = hi << 8 | lo;
                6
            }
            0x60 => {
                bus.read(self.pc);
                bus.read(0x0100 | self.sp as u16);
                let pc = self.pull16(bus);
                bus.read(pc);
                self.pc = pc.wrapping_add(1);
                6
            }
            0x40 => {
                bus.read(self.pc);
                bus.read(0x0100 | self.sp as u16);
                self.p = self.pull(bus) & !B | U;
                self.pc = self.pull16(bus);
                6
            }
            0x00 => {
                bus.read(self.pc);
                self.pc = self.pc.wrapping_add(1);
                self.push16(bus, self.pc);
                self.push(bus, self.p | B | U);
                self.p |= I;
                self.pc = self.read16(bus, 0xFFFE);
                7
            }
            0x10 => self.branch(bus, self.p & N == 0),
            0x30 => self.branch(bus, self.p & N != 0),
            0x50 => self.branch(bus, self.p & V == 0),
            0x70 => self.branch(bus, self.p & V != 0),
            0x90 => self.branch(bus, self.p & C == 0),
            0xB0 => self.branch(bus, self.p & C != 0),
            0xD0 => self.branch(bus, self.p & Z == 0),
            0xF0 => self.branch(bus, self.p & Z != 0),
            0xEA | 0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => {
                bus.read(self.pc);
                2
            }
            0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => rd!(imm, 2, nop),
            0x04 | 0x44 | 0x64 => rd!(zp, 3, nop),
            0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => rd!(zpx, 4, nop),
            0x0C => rd!(abs, 4, nop),
            0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => rd!(abx, 4, nop),
            0x8B => {
                self.fetch(bus);
                2
            }
            0x93 => {
                let ptr = self.fetch(bus);
                let lo = bus.read(ptr as u16);
                let hi = bus.read(ptr.wrapping_add(1) as u16);
                let base = (hi as u16) << 8 | lo as u16;
                let value = self.a & self.x;
                self.sh_store(bus, base, self.y, value);
                6
            }
            0x9F => {
                let base = self.fetch16(bus);
                let value = self.a & self.x;
                self.sh_store(bus, base, self.y, value);
                5
            }
            0x9B => {
                let base = self.fetch16(bus);
                self.sp = self.a & self.x;
                let value = self.sp;
                self.sh_store(bus, base, self.y, value);
                5
            }
            0xBB => {
                let (addr, crossed) = self.aby(bus);
                let v = bus.read(addr) & self.sp;
                self.a = v;
                self.x = v;
                self.sp = v;
                self.set_zn(v);
                4 + crossed as u32
            }
            0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
                bus.read(self.pc);
                2
            }
        }
    }

    fn fetch(&mut self, bus: &mut impl Bus) -> u8 {
        let v = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    fn sh_store(&mut self, bus: &mut impl Bus, base: u16, index: u8, reg: u8) {
        let addr = base.wrapping_add(index as u16);
        bus.read(base & 0xFF00 | addr & 0x00FF);
        let value = reg & ((base >> 8) as u8).wrapping_add(1);
        let addr = if addr & 0xFF00 != base & 0xFF00 {
            (value as u16) << 8 | addr & 0x00FF
        } else {
            addr
        };
        bus.write(addr, value);
    }

    fn fetch16(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.fetch(bus) as u16;
        let hi = self.fetch(bus) as u16;
        hi << 8 | lo
    }

    fn read16(&mut self, bus: &mut impl Bus, addr: u16) -> u16 {
        let lo = bus.read(addr) as u16;
        let hi = bus.read(addr.wrapping_add(1)) as u16;
        hi << 8 | lo
    }

    fn push(&mut self, bus: &mut impl Bus, v: u8) {
        bus.write(0x0100 | self.sp as u16, v);
        self.sp = self.sp.wrapping_sub(1);
    }

    fn push16(&mut self, bus: &mut impl Bus, v: u16) {
        self.push(bus, (v >> 8) as u8);
        self.push(bus, v as u8);
    }

    fn pull(&mut self, bus: &mut impl Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        bus.read(0x0100 | self.sp as u16)
    }

    fn pull16(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.pull(bus) as u16;
        let hi = self.pull(bus) as u16;
        hi << 8 | lo
    }

    fn interrupt(&mut self, bus: &mut impl Bus, vector: u16) {
        bus.read(self.pc);
        bus.read(self.pc);
        self.push16(bus, self.pc);
        self.push(bus, self.p & !B | U);
        self.p |= I;
        self.pc = self.read16(bus, vector);
    }

    fn set_flag(&mut self, f: u8, on: bool) {
        if on {
            self.p |= f;
        } else {
            self.p &= !f;
        }
    }

    fn set_zn(&mut self, v: u8) {
        self.set_flag(Z, v == 0);
        self.set_flag(N, v & 0x80 != 0);
    }

    fn imm(&mut self, _bus: &mut impl Bus) -> (u16, bool) {
        let addr = self.pc;
        self.pc = self.pc.wrapping_add(1);
        (addr, false)
    }

    fn zp(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        (self.fetch(bus) as u16, false)
    }

    fn zpx(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        let base = self.fetch(bus);
        bus.read(base as u16);
        (base.wrapping_add(self.x) as u16, false)
    }

    fn zpy(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        let base = self.fetch(bus);
        bus.read(base as u16);
        (base.wrapping_add(self.y) as u16, false)
    }

    fn abs(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        (self.fetch16(bus), false)
    }

    fn abx(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        let base = self.fetch16(bus);
        let addr = base.wrapping_add(self.x as u16);
        let crossed = addr & 0xFF00 != base & 0xFF00;
        if crossed {
            bus.read(base & 0xFF00 | addr & 0x00FF);
        }
        (addr, crossed)
    }

    fn abx_w(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        let base = self.fetch16(bus);
        let addr = base.wrapping_add(self.x as u16);
        bus.read(base & 0xFF00 | addr & 0x00FF);
        (addr, false)
    }

    fn aby(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        let base = self.fetch16(bus);
        let addr = base.wrapping_add(self.y as u16);
        let crossed = addr & 0xFF00 != base & 0xFF00;
        if crossed {
            bus.read(base & 0xFF00 | addr & 0x00FF);
        }
        (addr, crossed)
    }

    fn aby_w(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        let base = self.fetch16(bus);
        let addr = base.wrapping_add(self.y as u16);
        bus.read(base & 0xFF00 | addr & 0x00FF);
        (addr, false)
    }

    fn izx(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        let base = self.fetch(bus);
        bus.read(base as u16);
        let p = base.wrapping_add(self.x);
        let lo = bus.read(p as u16) as u16;
        let hi = bus.read(p.wrapping_add(1) as u16) as u16;
        (hi << 8 | lo, false)
    }

    fn izy(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        let p = self.fetch(bus);
        let lo = bus.read(p as u16) as u16;
        let hi = bus.read(p.wrapping_add(1) as u16) as u16;
        let base = hi << 8 | lo;
        let addr = base.wrapping_add(self.y as u16);
        let crossed = addr & 0xFF00 != base & 0xFF00;
        if crossed {
            bus.read(base & 0xFF00 | addr & 0x00FF);
        }
        (addr, crossed)
    }

    fn izy_w(&mut self, bus: &mut impl Bus) -> (u16, bool) {
        let p = self.fetch(bus);
        let lo = bus.read(p as u16) as u16;
        let hi = bus.read(p.wrapping_add(1) as u16) as u16;
        let base = hi << 8 | lo;
        let addr = base.wrapping_add(self.y as u16);
        bus.read(base & 0xFF00 | addr & 0x00FF);
        (addr, false)
    }

    fn branch(&mut self, bus: &mut impl Bus, cond: bool) -> u32 {
        let off = self.fetch(bus) as i8;
        if !cond {
            return 2;
        }
        bus.read(self.pc);
        let target = self.pc.wrapping_add(off as u16);
        let cycles = if target & 0xFF00 != self.pc & 0xFF00 {
            bus.read(self.pc & 0xFF00 | target & 0x00FF);
            4
        } else {
            3
        };
        self.pc = target;
        cycles
    }

    fn lda(&mut self, m: u8) {
        self.a = m;
        self.set_zn(m);
    }

    fn ldx(&mut self, m: u8) {
        self.x = m;
        self.set_zn(m);
    }

    fn ldy(&mut self, m: u8) {
        self.y = m;
        self.set_zn(m);
    }

    fn lax(&mut self, m: u8) {
        self.a = m;
        self.x = m;
        self.set_zn(m);
    }

    fn and(&mut self, m: u8) {
        self.a &= m;
        self.set_zn(self.a);
    }

    fn ora(&mut self, m: u8) {
        self.a |= m;
        self.set_zn(self.a);
    }

    fn eor(&mut self, m: u8) {
        self.a ^= m;
        self.set_zn(self.a);
    }

    fn bit(&mut self, m: u8) {
        self.set_flag(Z, self.a & m == 0);
        self.set_flag(V, m & V != 0);
        self.set_flag(N, m & N != 0);
    }

    fn adc(&mut self, m: u8) {
        let sum = self.a as u16 + m as u16 + (self.p & C) as u16;
        let r = sum as u8;
        self.set_flag(C, sum > 0xFF);
        self.set_flag(V, (self.a ^ r) & (m ^ r) & 0x80 != 0);
        self.a = r;
        self.set_zn(r);
    }

    fn sbc(&mut self, m: u8) {
        self.adc(!m);
    }

    fn compare(&mut self, reg: u8, m: u8) {
        self.set_flag(C, reg >= m);
        self.set_zn(reg.wrapping_sub(m));
    }

    fn cmp(&mut self, m: u8) {
        self.compare(self.a, m);
    }

    fn cpx(&mut self, m: u8) {
        self.compare(self.x, m);
    }

    fn cpy(&mut self, m: u8) {
        self.compare(self.y, m);
    }

    fn anc(&mut self, m: u8) {
        self.a &= m;
        self.set_zn(self.a);
        self.set_flag(C, self.a & 0x80 != 0);
    }

    fn alr(&mut self, m: u8) {
        let t = self.a & m;
        self.a = self.lsr(t);
    }

    fn arr(&mut self, m: u8) {
        let t = (self.a & m) >> 1 | (self.p & C) << 7;
        self.set_zn(t);
        self.set_flag(C, t & 0x40 != 0);
        self.set_flag(V, (t >> 6 ^ t >> 5) & 1 != 0);
        self.a = t;
    }

    fn axs(&mut self, m: u8) {
        let t = self.a & self.x;
        self.set_flag(C, t >= m);
        self.x = t.wrapping_sub(m);
        self.set_zn(self.x);
    }

    fn nop(&mut self, _m: u8) {}

    fn asl(&mut self, v: u8) -> u8 {
        self.set_flag(C, v & 0x80 != 0);
        let r = v << 1;
        self.set_zn(r);
        r
    }

    fn lsr(&mut self, v: u8) -> u8 {
        self.set_flag(C, v & 0x01 != 0);
        let r = v >> 1;
        self.set_zn(r);
        r
    }

    fn rol(&mut self, v: u8) -> u8 {
        let r = v << 1 | self.p & C;
        self.set_flag(C, v & 0x80 != 0);
        self.set_zn(r);
        r
    }

    fn ror(&mut self, v: u8) -> u8 {
        let r = v >> 1 | (self.p & C) << 7;
        self.set_flag(C, v & 0x01 != 0);
        self.set_zn(r);
        r
    }

    fn inc(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.set_zn(r);
        r
    }

    fn dec(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.set_zn(r);
        r
    }

    fn slo(&mut self, v: u8) -> u8 {
        let r = self.asl(v);
        self.ora(r);
        r
    }

    fn rla(&mut self, v: u8) -> u8 {
        let r = self.rol(v);
        self.and(r);
        r
    }

    fn sre(&mut self, v: u8) -> u8 {
        let r = self.lsr(v);
        self.eor(r);
        r
    }

    fn rra(&mut self, v: u8) -> u8 {
        let r = self.ror(v);
        self.adc(r);
        r
    }

    fn dcp(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.cmp(r);
        r
    }

    fn isb(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.sbc(r);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ram(Vec<u8>);

    impl Ram {
        fn new() -> Self {
            Ram(vec![0; 0x10000])
        }
    }

    impl Bus for Ram {
        fn read(&mut self, addr: u16) -> u8 {
            self.0[addr as usize]
        }

        fn write(&mut self, addr: u16, value: u8) {
            self.0[addr as usize] = value;
        }
    }

    fn exec(ram: &mut Ram, cpu: &mut Cpu, pc: u16, code: &[u8]) -> u32 {
        for (i, b) in code.iter().enumerate() {
            ram.0[pc as usize + i] = *b;
        }
        cpu.pc = pc;
        cpu.step(ram)
    }

    fn assert_flags(cpu: &Cpu, c: bool, z: bool, v: bool, n: bool) {
        assert_eq!(cpu.p & C != 0, c);
        assert_eq!(cpu.p & Z != 0, z);
        assert_eq!(cpu.p & V != 0, v);
        assert_eq!(cpu.p & N != 0, n);
    }

    #[test]
    fn adc_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for a in 0..=255u16 {
            for m in 0..=255u16 {
                for cin in 0..=1u16 {
                    cpu.a = a as u8;
                    cpu.p = U | cin as u8;
                    let cycles = exec(&mut ram, &mut cpu, 0x8000, &[0x69, m as u8]);
                    let sum = a + m + cin;
                    let r = sum as u8;
                    assert_eq!(cycles, 2);
                    assert_eq!(cpu.a, r);
                    let v = (a as u8 ^ r) & (m as u8 ^ r) & 0x80 != 0;
                    assert_flags(&cpu, sum > 0xFF, r == 0, v, r & 0x80 != 0);
                }
            }
        }
    }

    #[test]
    fn sbc_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for a in 0..=255i16 {
            for m in 0..=255i16 {
                for cin in 0..=1i16 {
                    for op in [0xE9u8, 0xEB] {
                        cpu.a = a as u8;
                        cpu.p = U | cin as u8;
                        let cycles = exec(&mut ram, &mut cpu, 0x8000, &[op, m as u8]);
                        let diff = a - m - (1 - cin);
                        let r = diff as u8;
                        assert_eq!(cycles, 2);
                        assert_eq!(cpu.a, r);
                        let v = (a as u8 ^ r) & (!(m as u8) ^ r) & 0x80 != 0;
                        assert_flags(&cpu, diff >= 0, r == 0, v, r & 0x80 != 0);

                        let mut adc_cpu = Cpu::new();
                        adc_cpu.a = a as u8;
                        adc_cpu.p = U | cin as u8;
                        let _ = exec(&mut ram, &mut adc_cpu, 0x8000, &[0x69, !(m as u8)]);
                        assert_eq!(adc_cpu.a, cpu.a);
                        assert_eq!(adc_cpu.p, cpu.p);
                    }
                }
            }
        }
    }

    #[test]
    fn compare_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for reg in 0..=255u8 {
            for m in 0..=255u8 {
                for op in [0xC9u8, 0xE0, 0xC0] {
                    cpu.a = 0;
                    cpu.x = 0;
                    cpu.y = 0;
                    match op {
                        0xC9 => cpu.a = reg,
                        0xE0 => cpu.x = reg,
                        _ => cpu.y = reg,
                    }
                    cpu.p = U;
                    let cycles = exec(&mut ram, &mut cpu, 0x8000, &[op, m]);
                    let r = reg.wrapping_sub(m);
                    assert_eq!(cycles, 2);
                    assert_flags(&cpu, reg >= m, reg == m, false, r & 0x80 != 0);
                    assert_eq!(
                        match op {
                            0xC9 => cpu.a,
                            0xE0 => cpu.x,
                            _ => cpu.y,
                        },
                        reg
                    );
                }
            }
        }
    }

    #[test]
    fn shifts_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for v in 0..=255u8 {
            for cin in 0..=1u8 {
                let cases: [(u8, u8, u8, bool); 4] = [
                    (0x0A, 0x06, v << 1, v & 0x80 != 0),
                    (0x4A, 0x46, v >> 1, v & 0x01 != 0),
                    (0x2A, 0x26, v << 1 | cin, v & 0x80 != 0),
                    (0x6A, 0x66, v >> 1 | cin << 7, v & 0x01 != 0),
                ];
                for (acc_op, mem_op, r, cout) in cases {
                    cpu.a = v;
                    cpu.p = U | cin;
                    let cycles = exec(&mut ram, &mut cpu, 0x8000, &[acc_op]);
                    assert_eq!(cycles, 2);
                    assert_eq!(cpu.a, r);
                    assert_flags(&cpu, cout, r == 0, false, r & 0x80 != 0);

                    ram.0[0x10] = v;
                    cpu.p = U | cin;
                    let cycles = exec(&mut ram, &mut cpu, 0x8000, &[mem_op, 0x10]);
                    assert_eq!(cycles, 5);
                    assert_eq!(ram.0[0x10], r);
                    assert_flags(&cpu, cout, r == 0, false, r & 0x80 != 0);
                }
            }
        }
    }

    #[test]
    fn logic_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for a in 0..=255u8 {
            for m in 0..=255u8 {
                let cases: [(u8, u8); 3] = [(0x29, a & m), (0x09, a | m), (0x49, a ^ m)];
                for (op, r) in cases {
                    cpu.a = a;
                    cpu.p = U;
                    let cycles = exec(&mut ram, &mut cpu, 0x8000, &[op, m]);
                    assert_eq!(cycles, 2);
                    assert_eq!(cpu.a, r);
                    assert_flags(&cpu, false, r == 0, false, r & 0x80 != 0);
                }
                cpu.a = a;
                cpu.p = U;
                ram.0[0x10] = m;
                let cycles = exec(&mut ram, &mut cpu, 0x8000, &[0x24, 0x10]);
                assert_eq!(cycles, 3);
                assert_eq!(cpu.a, a);
                assert_flags(&cpu, false, a & m == 0, m & 0x40 != 0, m & 0x80 != 0);
            }
        }
    }

    #[test]
    fn inc_dec_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for v in 0..=255u8 {
            for (op, r, cycles_expected) in [
                (0xE6u8, v.wrapping_add(1), 5u32),
                (0xC6, v.wrapping_sub(1), 5),
            ] {
                ram.0[0x10] = v;
                cpu.p = U;
                let cycles = exec(&mut ram, &mut cpu, 0x8000, &[op, 0x10]);
                assert_eq!(cycles, cycles_expected);
                assert_eq!(ram.0[0x10], r);
                assert_flags(&cpu, false, r == 0, false, r & 0x80 != 0);
            }
            for (op, up, uses_x) in [
                (0xE8u8, true, true),
                (0xC8, true, false),
                (0xCA, false, true),
                (0x88, false, false),
            ] {
                let expected = if up {
                    v.wrapping_add(1)
                } else {
                    v.wrapping_sub(1)
                };
                cpu.x = v;
                cpu.y = v;
                cpu.p = U;
                let cycles = exec(&mut ram, &mut cpu, 0x8000, &[op]);
                assert_eq!(cycles, 2);
                let got = if uses_x { cpu.x } else { cpu.y };
                assert_eq!(got, expected);
                assert_flags(&cpu, false, expected == 0, false, expected & 0x80 != 0);
            }
        }
    }

    #[test]
    fn stack_roundtrip_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for v in 0..=255u8 {
            cpu.sp = 0xFD;
            cpu.a = v;
            cpu.p = U;
            let c1 = exec(&mut ram, &mut cpu, 0x8000, &[0x48, 0x68]);
            assert_eq!(c1, 3);
            assert_eq!(cpu.sp, 0xFC);
            assert_eq!(ram.0[0x01FD], v);
            cpu.a = 0;
            let c2 = cpu.step(&mut ram);
            assert_eq!(c2, 4);
            assert_eq!(cpu.sp, 0xFD);
            assert_eq!(cpu.a, v);
            assert_flags(&cpu, false, v == 0, false, v & 0x80 != 0);

            cpu.sp = 0xFD;
            cpu.p = v;
            let c3 = exec(&mut ram, &mut cpu, 0x8000, &[0x08, 0x28]);
            assert_eq!(c3, 3);
            assert_eq!(ram.0[0x01FD], v | 0x30);
            cpu.p = 0;
            let c4 = cpu.step(&mut ram);
            assert_eq!(c4, 4);
            assert_eq!(cpu.p, (v | 0x30) & !B | U);

            cpu.sp = 0xFC;
            ram.0[0x01FD] = v;
            let c5 = exec(&mut ram, &mut cpu, 0x8000, &[0x28]);
            assert_eq!(c5, 4);
            assert_eq!(cpu.p, v & !B | U);
        }
    }

    #[test]
    fn zero_page_x_wrap_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for i in 0..=255u8 {
            ram.0[i as usize] = i;
        }
        for base in 0..=255u8 {
            for x in 0..=255u8 {
                cpu.x = x;
                let cycles = exec(&mut ram, &mut cpu, 0x8000, &[0xB5, base]);
                assert_eq!(cycles, 4);
                assert_eq!(cpu.a, base.wrapping_add(x));
            }
        }
    }

    #[test]
    fn indexed_indirect_wrap_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for base in 0..=255u8 {
            for x in 0..=255u8 {
                let p = base.wrapping_add(x);
                let target = 0x0200u16 | p as u16;
                ram.0[p as usize] = p;
                ram.0[p.wrapping_add(1) as usize] = 0x02;
                ram.0[target as usize] = p ^ 0xA5;
                cpu.x = x;
                let cycles = exec(&mut ram, &mut cpu, 0x8000, &[0xA1, base]);
                assert_eq!(cycles, 6);
                assert_eq!(cpu.a, p ^ 0xA5);
            }
        }
    }

    #[test]
    fn indirect_indexed_wrap_and_penalty_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for zp in 0..=255u8 {
            for y in 0..=255u8 {
                ram.0[zp as usize] = 0x80;
                ram.0[zp.wrapping_add(1) as usize] = 0x40;
                let target = 0x4080u16 + y as u16;
                ram.0[target as usize] = y ^ 0x5A;
                cpu.y = y;
                let cycles = exec(&mut ram, &mut cpu, 0x8000, &[0xB1, zp]);
                let crossed = 0x80u16 + y as u16 > 0xFF;
                assert_eq!(cycles, 5 + crossed as u32);
                assert_eq!(cpu.a, y ^ 0x5A);
            }
        }
    }

    #[test]
    fn absolute_x_page_cross_penalty() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for base in [0x1000u16, 0x10F8, 0x20FF, 0x3380, 0x44C0] {
            for x in 0..=255u8 {
                let addr = base.wrapping_add(x as u16);
                let marker = (addr as u8) ^ (addr >> 8) as u8;
                ram.0[addr as usize] = marker;
                cpu.x = x;
                let code = [0xBD, base as u8, (base >> 8) as u8];
                let cycles = exec(&mut ram, &mut cpu, 0x8000, &code);
                let crossed = addr & 0xFF00 != base & 0xFF00;
                assert_eq!(cycles, 4 + crossed as u32);
                assert_eq!(cpu.a, marker);
            }
        }
    }

    #[test]
    fn branch_cycles_exhaustive() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for pc0 in 0x4000u16..0x4200 {
            for disp in 0..=255u8 {
                cpu.p = U;
                let cycles = exec(&mut ram, &mut cpu, pc0, &[0xF0, disp]);
                assert_eq!(cycles, 2);
                assert_eq!(cpu.pc, pc0.wrapping_add(2));

                cpu.p = U | Z;
                let cycles = exec(&mut ram, &mut cpu, pc0, &[0xF0, disp]);
                let post = pc0.wrapping_add(2);
                let target = post.wrapping_add(disp as i8 as u16);
                let expected = if target & 0xFF00 != post & 0xFF00 {
                    4
                } else {
                    3
                };
                assert_eq!(cycles, expected);
                assert_eq!(cpu.pc, target);
            }
        }
    }

    #[test]
    fn jmp_indirect_page_wrap_bug() {
        let mut ram = Ram::new();
        let mut cpu = Cpu::new();
        for page in 0..=255u16 {
            let ptr = page << 8 | 0xFF;
            ram.0[ptr as usize] = 0x55;
            if page != 0x80 {
                ram.0[(page << 8) as usize] = 0xAA;
            }
            let hi = ram.0[(page << 8) as usize] as u16;
            let cycles = exec(&mut ram, &mut cpu, 0x8000, &[0x6C, 0xFF, page as u8]);
            assert_eq!(cycles, 5);
            assert_eq!(cpu.pc, hi << 8 | 0x55);
        }
    }

    #[test]
    fn nmi_pushes_state_and_jumps() {
        for p in 0..=255u8 {
            let mut ram = Ram::new();
            ram.0[0xFFFA] = 0x34;
            ram.0[0xFFFB] = 0x12;
            let mut cpu = Cpu::new();
            cpu.pc = 0xBEEF;
            cpu.p = p;
            cpu.sp = 0xFD;
            cpu.nmi_pending = true;
            let cycles = cpu.step(&mut ram);
            assert_eq!(cycles, 7);
            assert!(!cpu.nmi_pending);
            assert_eq!(ram.0[0x01FD], 0xBE);
            assert_eq!(ram.0[0x01FC], 0xEF);
            assert_eq!(ram.0[0x01FB], p & !B | U);
            assert_eq!(cpu.sp, 0xFA);
            assert_eq!(cpu.pc, 0x1234);
            assert_ne!(cpu.p & I, 0);
        }
    }

    #[test]
    fn irq_respects_mask_and_vector() {
        let mut ram = Ram::new();
        ram.0[0xFFFE] = 0x78;
        ram.0[0xFFFF] = 0x56;
        ram.0[0xC000] = 0xEA;
        let mut cpu = Cpu::new();
        cpu.pc = 0xC000;
        cpu.p = U | I;
        cpu.irq_line = true;
        let cycles = cpu.step(&mut ram);
        assert_eq!(cycles, 2);
        assert_eq!(cpu.pc, 0xC001);
        assert!(cpu.irq_line);

        cpu.pc = 0xC000;
        cpu.p = U | B;
        cpu.sp = 0xFD;
        let cycles = cpu.step(&mut ram);
        assert_eq!(cycles, 7);
        assert_eq!(ram.0[0x01FD], 0xC0);
        assert_eq!(ram.0[0x01FC], 0x00);
        assert_eq!(ram.0[0x01FB], U);
        assert_eq!(cpu.pc, 0x5678);
        assert_ne!(cpu.p & I, 0);

        let mut cpu = Cpu::new();
        cpu.pc = 0xC000;
        cpu.p = U;
        cpu.irq_line = true;
        cpu.nmi_pending = true;
        ram.0[0xFFFA] = 0x00;
        ram.0[0xFFFB] = 0x90;
        cpu.step(&mut ram);
        assert_eq!(cpu.pc, 0x9000);
    }

    struct Rec {
        mem: Vec<u8>,
        log: Vec<(u16, u8, bool)>,
    }

    impl Rec {
        fn new(fill: u8) -> Self {
            Rec {
                mem: vec![fill; 0x10000],
                log: Vec::new(),
            }
        }
    }

    impl Bus for Rec {
        fn read(&mut self, addr: u16) -> u8 {
            let v = self.mem[addr as usize];
            self.log.push((addr, v, false));
            v
        }

        fn write(&mut self, addr: u16, value: u8) {
            self.mem[addr as usize] = value;
            self.log.push((addr, value, true));
        }
    }

    #[test]
    fn bus_access_count_matches_cycles_all_opcodes() {
        for op in 0..=255u8 {
            for (fill, x, y, p) in [
                (0x11u8, 0u8, 0u8, U),
                (0x80, 0xFF, 0xFF, U | C | Z | V | N),
                (0xFE, 0x05, 0xFA, U | C),
            ] {
                let mut bus = Rec::new(fill);
                let mut cpu = Cpu::new();
                cpu.pc = 0x4000;
                cpu.sp = 0xFD;
                cpu.a = 0x3C;
                cpu.x = x;
                cpu.y = y;
                cpu.p = p;
                bus.mem[0x4000] = op;
                let cycles = cpu.step(&mut bus);
                assert_eq!(bus.log.len() as u32, cycles, "op {op:02X} fill {fill:02X}");
            }
        }
        for (nmi, irq) in [(true, false), (false, true)] {
            let mut bus = Rec::new(0x22);
            let mut cpu = Cpu::new();
            cpu.pc = 0x4000;
            cpu.p = U;
            cpu.nmi_pending = nmi;
            cpu.irq_line = irq;
            let cycles = cpu.step(&mut bus);
            assert_eq!(cycles, 7);
            assert_eq!(bus.log.len(), 7);
        }
    }

    #[test]
    fn sta_abx_bus_pattern() {
        for x in 0..=255u8 {
            let mut bus = Rec::new(0);
            let mut cpu = Cpu::new();
            cpu.pc = 0x4000;
            cpu.a = 0x5A;
            cpu.x = x;
            bus.mem[0x4000] = 0x9D;
            bus.mem[0x4001] = 0xC0;
            bus.mem[0x4002] = 0x12;
            let cycles = cpu.step(&mut bus);
            let base = 0x12C0u16;
            let addr = base.wrapping_add(x as u16);
            let partial = base & 0xFF00 | addr & 0x00FF;
            assert_eq!(cycles, 5);
            assert_eq!(
                bus.log,
                vec![
                    (0x4000, 0x9D, false),
                    (0x4001, 0xC0, false),
                    (0x4002, 0x12, false),
                    (partial, 0, false),
                    (addr, 0x5A, true),
                ]
            );
        }
    }

    #[test]
    fn lda_abx_bus_pattern() {
        for x in 0..=255u8 {
            let mut bus = Rec::new(0x33);
            let mut cpu = Cpu::new();
            cpu.pc = 0x4000;
            cpu.x = x;
            bus.mem[0x4000] = 0xBD;
            bus.mem[0x4001] = 0xC0;
            bus.mem[0x4002] = 0x12;
            let base = 0x12C0u16;
            let addr = base.wrapping_add(x as u16);
            bus.mem[addr as usize] = x ^ 0x77;
            let cycles = cpu.step(&mut bus);
            let crossed = addr & 0xFF00 != base & 0xFF00;
            assert_eq!(cycles, 4 + crossed as u32);
            assert_eq!(cpu.a, x ^ 0x77);
            let mut expected = vec![
                (0x4000u16, 0xBDu8, false),
                (0x4001, 0xC0, false),
                (0x4002, 0x12, false),
            ];
            if crossed {
                expected.push((base & 0xFF00 | addr & 0x00FF, 0x33, false));
            }
            expected.push((addr, x ^ 0x77, false));
            assert_eq!(bus.log, expected);
        }
    }

    #[test]
    fn rmw_zp_bus_pattern() {
        for v in 0..=255u8 {
            let mut bus = Rec::new(0);
            let mut cpu = Cpu::new();
            cpu.pc = 0x4000;
            bus.mem[0x4000] = 0xE6;
            bus.mem[0x4001] = 0x20;
            bus.mem[0x20] = v;
            let cycles = cpu.step(&mut bus);
            assert_eq!(cycles, 5);
            assert_eq!(
                bus.log,
                vec![
                    (0x4000, 0xE6, false),
                    (0x4001, 0x20, false),
                    (0x0020, v, false),
                    (0x0020, v, true),
                    (0x0020, v.wrapping_add(1), true),
                ]
            );
        }
    }

    #[test]
    fn branch_bus_pattern_sweep() {
        for pc0 in [0x4010u16, 0x40F0] {
            for disp in 0..=255u8 {
                let mut bus = Rec::new(0);
                let mut cpu = Cpu::new();
                cpu.pc = pc0;
                cpu.p = U;
                bus.mem[pc0 as usize] = 0xF0;
                bus.mem[pc0 as usize + 1] = disp;
                let cycles = cpu.step(&mut bus);
                assert_eq!(cycles, 2);
                assert_eq!(bus.log.len(), 2);

                let mut bus = Rec::new(0);
                let mut cpu = Cpu::new();
                cpu.pc = pc0;
                cpu.p = U | Z;
                bus.mem[pc0 as usize] = 0xF0;
                bus.mem[pc0 as usize + 1] = disp;
                let cycles = cpu.step(&mut bus);
                let post = pc0.wrapping_add(2);
                let target = post.wrapping_add(disp as i8 as u16);
                let crossed = target & 0xFF00 != post & 0xFF00;
                assert_eq!(cycles, 3 + crossed as u32);
                assert_eq!(bus.log.len() as u32, cycles);
                assert_eq!(bus.log[2].0, post);
                assert!(!bus.log[2].2);
                if crossed {
                    assert_eq!(bus.log[3].0, post & 0xFF00 | target & 0x00FF);
                    assert!(!bus.log[3].2);
                }
                assert_eq!(cpu.pc, target);
            }
        }
    }

    #[test]
    fn zpx_and_izy_store_dummy_pattern() {
        for base in 0..=255u8 {
            let mut bus = Rec::new(0);
            let mut cpu = Cpu::new();
            cpu.pc = 0x4000;
            cpu.x = 0x30;
            bus.mem[0x4000] = 0xB5;
            bus.mem[0x4001] = base;
            let cycles = cpu.step(&mut bus);
            assert_eq!(cycles, 4);
            assert_eq!(bus.log[2], (base as u16, 0, false));
            assert_eq!(bus.log[3].0, base.wrapping_add(0x30) as u16);
            assert!(!bus.log[3].2);

            let mut bus = Rec::new(0);
            let mut cpu = Cpu::new();
            cpu.pc = 0x4000;
            cpu.a = 0xAB;
            cpu.y = 0xFF;
            bus.mem[0x4000] = 0x91;
            bus.mem[0x4001] = base;
            bus.mem[base as usize] = 0x80;
            bus.mem[base.wrapping_add(1) as usize] = 0x40;
            let cycles = cpu.step(&mut bus);
            assert_eq!(cycles, 6);
            assert_eq!(
                bus.log[2..],
                [
                    (base as u16, 0x80, false),
                    (base.wrapping_add(1) as u16, 0x40, false),
                    (0x407F, 0, false),
                    (0x417F, 0xAB, true),
                ]
            );
        }
    }

    #[test]
    fn save_load_roundtrip() {
        let mut cpu = Cpu::new();
        cpu.a = 0x12;
        cpu.x = 0x34;
        cpu.y = 0x56;
        cpu.sp = 0x78;
        cpu.pc = 0x9ABC;
        cpu.p = 0xDE;
        cpu.nmi_pending = true;
        cpu.irq_line = true;
        let mut w = Writer::new();
        cpu.save(&mut w);
        let mut other = Cpu::new();
        let mut r = Reader::new(&w.data);
        other.load(&mut r);
        assert!(r.done());
        assert_eq!(other.a, 0x12);
        assert_eq!(other.x, 0x34);
        assert_eq!(other.y, 0x56);
        assert_eq!(other.sp, 0x78);
        assert_eq!(other.pc, 0x9ABC);
        assert_eq!(other.p, 0xDE);
        assert!(other.nmi_pending);
        assert!(other.irq_line);
    }
}
