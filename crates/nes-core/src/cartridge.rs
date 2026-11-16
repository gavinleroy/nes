use crate::state::{Reader, Writer};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    SingleLower,
    SingleUpper,
    FourScreen,
}

fn mirroring_to_u8(m: Mirroring) -> u8 {
    match m {
        Mirroring::Horizontal => 0,
        Mirroring::Vertical => 1,
        Mirroring::SingleLower => 2,
        Mirroring::SingleUpper => 3,
        Mirroring::FourScreen => 4,
    }
}

fn mirroring_from_u8(v: u8) -> Mirroring {
    match v {
        1 => Mirroring::Vertical,
        2 => Mirroring::SingleLower,
        3 => Mirroring::SingleUpper,
        4 => Mirroring::FourScreen,
        _ => Mirroring::Horizontal,
    }
}

fn nrom_prg_index(addr: u16, prg_len: usize) -> usize {
    (addr as usize & 0x7FFF) % prg_len
}

fn nrom_offsets(prg_len: usize) -> [usize; 2] {
    [
        nrom_prg_index(0x8000, prg_len),
        nrom_prg_index(0xC000, prg_len),
    ]
}

fn uxrom_prg_index(addr: u16, bank: u8, banks: usize) -> usize {
    let sel = if addr < 0xC000 {
        (bank as usize & 0x0F) % banks
    } else {
        banks - 1
    };
    sel * 0x4000 + (addr as usize & 0x3FFF)
}

fn cnrom_chr_index(addr: u16, bank: u8, banks: usize) -> usize {
    ((bank as usize & 3) % banks) * 0x2000 + (addr as usize & 0x1FFF)
}

fn mmc1_prg_index(addr: u16, control: u8, bank: u8, prg_len: usize) -> usize {
    let banks = prg_len / 0x4000;
    let sel = bank as usize % banks;
    match (control >> 2) & 3 {
        0 | 1 => ((sel & !1) * 0x4000 + (addr as usize & 0x7FFF)) % prg_len,
        2 => {
            if addr < 0xC000 {
                addr as usize & 0x3FFF
            } else {
                sel * 0x4000 + (addr as usize & 0x3FFF)
            }
        }
        _ => {
            if addr < 0xC000 {
                sel * 0x4000 + (addr as usize & 0x3FFF)
            } else {
                prg_len - 0x4000 + (addr as usize & 0x3FFF)
            }
        }
    }
}

fn mmc1_chr_index(addr: u16, control: u8, chr0: u8, chr1: u8, chr_len: usize) -> usize {
    let banks = chr_len / 0x1000;
    if control & 0x10 != 0 {
        let sel = if addr & 0x1FFF < 0x1000 { chr0 } else { chr1 } as usize % banks;
        sel * 0x1000 + (addr as usize & 0x0FFF)
    } else {
        let sel = (chr0 as usize % banks) & !1;
        (sel * 0x1000 + (addr as usize & 0x1FFF)) % chr_len
    }
}

fn mmc3_prg_index(addr: u16, prg_mode: bool, r6: u8, r7: u8, prg_len: usize) -> usize {
    let banks = prg_len / 0x2000;
    let sel = match ((addr >> 13) & 3, prg_mode) {
        (0, false) => r6 as usize % banks,
        (0, true) => banks - 2,
        (1, _) => r7 as usize % banks,
        (2, false) => banks - 2,
        (2, true) => r6 as usize % banks,
        _ => banks - 1,
    };
    sel * 0x2000 + (addr as usize & 0x1FFF)
}

fn mmc3_chr_index(addr: u16, chr_inversion: bool, regs: &[u8; 8], chr_len: usize) -> usize {
    let banks = chr_len / 0x400;
    let window = ((addr as usize >> 10) & 7) ^ if chr_inversion { 4 } else { 0 };
    let sel = match window {
        0 => regs[0] as usize & !1,
        1 => regs[0] as usize | 1,
        2 => regs[1] as usize & !1,
        3 => regs[1] as usize | 1,
        w => regs[w - 2] as usize,
    };
    (sel % banks) * 0x400 + (addr as usize & 0x3FF)
}

struct Common {
    prg: Vec<u8>,
    chr: Vec<u8>,
    chr_writable: bool,
    prg_ram: Vec<u8>,
}

impl Common {
    fn ram_read(&self, addr: u16) -> u8 {
        self.prg_ram[(addr as usize) & 0x1FFF]
    }

    fn ram_write(&mut self, addr: u16, value: u8) {
        self.prg_ram[(addr as usize) & 0x1FFF] = value;
    }

    fn chr_write(&mut self, index: usize, value: u8) {
        if self.chr_writable {
            self.chr[index] = value;
        }
    }

    fn save(&self, w: &mut Writer) {
        w.bytes(&self.prg_ram);
        if self.chr_writable {
            w.bytes(&self.chr);
        }
    }

    fn load(&mut self, r: &mut Reader) {
        r.bytes(&mut self.prg_ram);
        if self.chr_writable {
            r.bytes(&mut self.chr);
        }
    }
}

enum MapperKind {
    Nrom(Nrom),
    Mmc1(Mmc1),
    Uxrom(Uxrom),
    Cnrom(Cnrom),
    Mmc3(Mmc3),
}

struct Nrom {
    c: Common,
    mirror: Mirroring,
    prg_off: [usize; 2],
}

impl Nrom {
    fn new(c: Common, mirror: Mirroring) -> Nrom {
        let prg_off = nrom_offsets(c.prg.len());
        Nrom { c, mirror, prg_off }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.c.prg[self.prg_off[(addr as usize >> 14) & 1] + (addr as usize & 0x3FFF)]
        } else if addr >= 0x6000 {
            self.c.ram_read(addr)
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, value: u8) {
        if let 0x6000..=0x7FFF = addr {
            self.c.ram_write(addr, value);
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.c.chr[addr as usize & 0x1FFF]
    }

    fn ppu_write(&mut self, addr: u16, value: u8) {
        let index = addr as usize & 0x1FFF;
        self.c.chr_write(index, value);
    }

    fn mirroring(&self) -> Mirroring {
        self.mirror
    }

    fn save(&self, w: &mut Writer) {
        self.c.save(w);
    }

    fn load(&mut self, r: &mut Reader) {
        self.c.load(r);
    }
}

struct Uxrom {
    c: Common,
    mirror: Mirroring,
    bank: u8,
    prg_off: [usize; 2],
}

impl Uxrom {
    fn new(c: Common, mirror: Mirroring) -> Uxrom {
        let mut m = Uxrom {
            c,
            mirror,
            bank: 0,
            prg_off: [0; 2],
        };
        m.update_banks();
        m
    }

    fn update_banks(&mut self) {
        let banks = self.c.prg.len() / 0x4000;
        self.prg_off[0] = uxrom_prg_index(0x8000, self.bank, banks);
        self.prg_off[1] = uxrom_prg_index(0xC000, self.bank, banks);
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.c.prg[self.prg_off[(addr as usize >> 14) & 1] + (addr as usize & 0x3FFF)]
        } else if addr >= 0x6000 {
            self.c.ram_read(addr)
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, value: u8) {
        match addr {
            0x6000..=0x7FFF => self.c.ram_write(addr, value),
            0x8000..=0xFFFF => {
                self.bank = value & 0x0F;
                self.update_banks();
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.c.chr[addr as usize & 0x1FFF]
    }

    fn ppu_write(&mut self, addr: u16, value: u8) {
        let index = addr as usize & 0x1FFF;
        self.c.chr_write(index, value);
    }

    fn mirroring(&self) -> Mirroring {
        self.mirror
    }

    fn save(&self, w: &mut Writer) {
        w.u8(self.bank);
        self.c.save(w);
    }

    fn load(&mut self, r: &mut Reader) {
        self.bank = r.u8();
        self.c.load(r);
        self.update_banks();
    }
}

struct Cnrom {
    c: Common,
    mirror: Mirroring,
    bank: u8,
    prg_off: [usize; 2],
    chr_base: usize,
}

impl Cnrom {
    fn new(c: Common, mirror: Mirroring) -> Cnrom {
        let prg_off = nrom_offsets(c.prg.len());
        let mut m = Cnrom {
            c,
            mirror,
            bank: 0,
            prg_off,
            chr_base: 0,
        };
        m.update_banks();
        m
    }

    fn update_banks(&mut self) {
        self.chr_base = cnrom_chr_index(0, self.bank, self.c.chr.len() / 0x2000);
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.c.prg[self.prg_off[(addr as usize >> 14) & 1] + (addr as usize & 0x3FFF)]
        } else if addr >= 0x6000 {
            self.c.ram_read(addr)
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, value: u8) {
        match addr {
            0x6000..=0x7FFF => self.c.ram_write(addr, value),
            0x8000..=0xFFFF => {
                self.bank = value & 3;
                self.update_banks();
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.c.chr[self.chr_base + (addr as usize & 0x1FFF)]
    }

    fn ppu_write(&mut self, addr: u16, value: u8) {
        let index = self.chr_base + (addr as usize & 0x1FFF);
        self.c.chr_write(index, value);
    }

    fn mirroring(&self) -> Mirroring {
        self.mirror
    }

    fn save(&self, w: &mut Writer) {
        w.u8(self.bank);
        self.c.save(w);
    }

    fn load(&mut self, r: &mut Reader) {
        self.bank = r.u8();
        self.c.load(r);
        self.update_banks();
    }
}

struct Mmc1Regs {
    shift: u8,
    count: u8,
    control: u8,
    chr0: u8,
    chr1: u8,
    prg: u8,
}

impl Mmc1Regs {
    fn new() -> Mmc1Regs {
        Mmc1Regs {
            shift: 0,
            count: 0,
            control: 0x0C,
            chr0: 0,
            chr1: 0,
            prg: 0,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if value & 0x80 != 0 {
            self.shift = 0;
            self.count = 0;
            self.control |= 0x0C;
            return;
        }
        self.shift |= (value & 1) << self.count;
        self.count += 1;
        if self.count == 5 {
            let loaded = self.shift & 0x1F;
            match (addr >> 13) & 3 {
                0 => self.control = loaded,
                1 => self.chr0 = loaded,
                2 => self.chr1 = loaded,
                _ => self.prg = loaded & 0x0F,
            }
            self.shift = 0;
            self.count = 0;
        }
    }
}

struct Mmc1 {
    c: Common,
    r: Mmc1Regs,
    prg_off: [usize; 2],
    chr_off: [usize; 2],
}

impl Mmc1 {
    fn new(c: Common, r: Mmc1Regs) -> Mmc1 {
        let mut m = Mmc1 {
            c,
            r,
            prg_off: [0; 2],
            chr_off: [0; 2],
        };
        m.update_banks();
        m
    }

    fn update_banks(&mut self) {
        self.prg_off[0] = mmc1_prg_index(0x8000, self.r.control, self.r.prg, self.c.prg.len());
        self.prg_off[1] = mmc1_prg_index(0xC000, self.r.control, self.r.prg, self.c.prg.len());
        self.chr_off[0] = mmc1_chr_index(
            0,
            self.r.control,
            self.r.chr0,
            self.r.chr1,
            self.c.chr.len(),
        );
        self.chr_off[1] = mmc1_chr_index(
            0x1000,
            self.r.control,
            self.r.chr0,
            self.r.chr1,
            self.c.chr.len(),
        );
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.c.prg[self.prg_off[(addr as usize >> 14) & 1] + (addr as usize & 0x3FFF)]
        } else if addr >= 0x6000 {
            self.c.ram_read(addr)
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, value: u8) {
        match addr {
            0x6000..=0x7FFF => self.c.ram_write(addr, value),
            0x8000..=0xFFFF => {
                self.r.write(addr, value);
                self.update_banks();
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.c.chr[self.chr_off[(addr as usize >> 12) & 1] + (addr as usize & 0x0FFF)]
    }

    fn ppu_write(&mut self, addr: u16, value: u8) {
        let index = self.chr_off[(addr as usize >> 12) & 1] + (addr as usize & 0x0FFF);
        self.c.chr_write(index, value);
    }

    fn mirroring(&self) -> Mirroring {
        match self.r.control & 3 {
            0 => Mirroring::SingleLower,
            1 => Mirroring::SingleUpper,
            2 => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
        }
    }

    fn save(&self, w: &mut Writer) {
        w.u8(self.r.shift);
        w.u8(self.r.count);
        w.u8(self.r.control);
        w.u8(self.r.chr0);
        w.u8(self.r.chr1);
        w.u8(self.r.prg);
        self.c.save(w);
    }

    fn load(&mut self, r: &mut Reader) {
        self.r.shift = r.u8();
        self.r.count = r.u8();
        self.r.control = r.u8();
        self.r.chr0 = r.u8();
        self.r.chr1 = r.u8();
        self.r.prg = r.u8();
        self.c.load(r);
        self.update_banks();
    }
}

struct Mmc3Regs {
    select: u8,
    regs: [u8; 8],
    prg_mode: bool,
    chr_inversion: bool,
    mirror: Mirroring,
    irq_latch: u8,
    irq_counter: u8,
    irq_reload: bool,
    irq_enabled: bool,
    irq_pending: bool,
}

impl Mmc3Regs {
    fn new(mirror: Mirroring) -> Mmc3Regs {
        Mmc3Regs {
            select: 0,
            regs: [0; 8],
            prg_mode: false,
            chr_inversion: false,
            mirror,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload: false,
            irq_enabled: false,
            irq_pending: false,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr & 0x6001 {
            0x0000 => {
                self.select = value & 7;
                self.prg_mode = value & 0x40 != 0;
                self.chr_inversion = value & 0x80 != 0;
            }
            0x0001 => self.regs[self.select as usize] = value,
            0x2000 => {
                if self.mirror != Mirroring::FourScreen {
                    self.mirror = if value & 1 != 0 {
                        Mirroring::Horizontal
                    } else {
                        Mirroring::Vertical
                    };
                }
            }
            0x2001 => {}
            0x4000 => self.irq_latch = value,
            0x4001 => self.irq_reload = true,
            0x6000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            _ => self.irq_enabled = true,
        }
    }

    fn clock_irq(&mut self) {
        if self.irq_counter == 0 || self.irq_reload {
            self.irq_counter = self.irq_latch;
            self.irq_reload = false;
        } else {
            self.irq_counter -= 1;
        }
        if self.irq_counter == 0 && self.irq_enabled {
            self.irq_pending = true;
        }
    }
}

struct Mmc3 {
    c: Common,
    r: Mmc3Regs,
    prg_off: [usize; 4],
    chr_off: [usize; 8],
}

impl Mmc3 {
    fn new(c: Common, r: Mmc3Regs) -> Mmc3 {
        let mut m = Mmc3 {
            c,
            r,
            prg_off: [0; 4],
            chr_off: [0; 8],
        };
        m.update_banks();
        m
    }

    fn update_banks(&mut self) {
        for w in 0..4 {
            self.prg_off[w] = mmc3_prg_index(
                0x8000 + (w as u16) * 0x2000,
                self.r.prg_mode,
                self.r.regs[6],
                self.r.regs[7],
                self.c.prg.len(),
            );
        }
        for w in 0..8 {
            self.chr_off[w] = mmc3_chr_index(
                (w as u16) * 0x400,
                self.r.chr_inversion,
                &self.r.regs,
                self.c.chr.len(),
            );
        }
    }

    fn cpu_read(&mut self, addr: u16) -> u8 {
        if addr >= 0x8000 {
            self.c.prg[self.prg_off[(addr as usize >> 13) & 3] + (addr as usize & 0x1FFF)]
        } else if addr >= 0x6000 {
            self.c.ram_read(addr)
        } else {
            0
        }
    }

    fn cpu_write(&mut self, addr: u16, value: u8) {
        match addr {
            0x6000..=0x7FFF => self.c.ram_write(addr, value),
            0x8000..=0xFFFF => {
                self.r.write(addr, value);
                self.update_banks();
            }
            _ => {}
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        self.c.chr[self.chr_off[(addr as usize >> 10) & 7] + (addr as usize & 0x3FF)]
    }

    fn ppu_write(&mut self, addr: u16, value: u8) {
        let index = self.chr_off[(addr as usize >> 10) & 7] + (addr as usize & 0x3FF);
        self.c.chr_write(index, value);
    }

    fn mirroring(&self) -> Mirroring {
        self.r.mirror
    }

    fn a12(&mut self) {
        self.r.clock_irq();
    }

    fn save(&self, w: &mut Writer) {
        w.u8(self.r.select);
        w.bytes(&self.r.regs);
        w.bool(self.r.prg_mode);
        w.bool(self.r.chr_inversion);
        w.u8(mirroring_to_u8(self.r.mirror));
        w.u8(self.r.irq_latch);
        w.u8(self.r.irq_counter);
        w.bool(self.r.irq_reload);
        w.bool(self.r.irq_enabled);
        w.bool(self.r.irq_pending);
        self.c.save(w);
    }

    fn load(&mut self, r: &mut Reader) {
        self.r.select = r.u8();
        r.bytes(&mut self.r.regs);
        self.r.prg_mode = r.bool();
        self.r.chr_inversion = r.bool();
        self.r.mirror = mirroring_from_u8(r.u8());
        self.r.irq_latch = r.u8();
        self.r.irq_counter = r.u8();
        self.r.irq_reload = r.bool();
        self.r.irq_enabled = r.bool();
        self.r.irq_pending = r.bool();
        self.c.load(r);
        self.update_banks();
    }
}

pub struct Cartridge {
    mapper: MapperKind,
    mapper_id: u8,
    four_screen: bool,
}

macro_rules! dispatch {
    ($self:expr, $m:ident => $body:expr) => {
        match &mut $self.mapper {
            MapperKind::Nrom($m) => $body,
            MapperKind::Mmc1($m) => $body,
            MapperKind::Uxrom($m) => $body,
            MapperKind::Cnrom($m) => $body,
            MapperKind::Mmc3($m) => $body,
        }
    };
    ($self:expr, ref $m:ident => $body:expr) => {
        match &$self.mapper {
            MapperKind::Nrom($m) => $body,
            MapperKind::Mmc1($m) => $body,
            MapperKind::Uxrom($m) => $body,
            MapperKind::Cnrom($m) => $body,
            MapperKind::Mmc3($m) => $body,
        }
    };
}

impl Cartridge {
    pub fn from_ines(data: &[u8]) -> Result<Cartridge, String> {
        if data.len() < 16 || &data[0..4] != b"NES\x1A" {
            return Err("invalid iNES header".to_string());
        }
        let prg_len = data[4] as usize * 0x4000;
        let chr_len = data[5] as usize * 0x2000;
        let flags6 = data[6];
        let flags7 = data[7];
        if prg_len == 0 {
            return Err("no PRG ROM".to_string());
        }
        let mapper_id = (flags7 & 0xF0) | (flags6 >> 4);
        let four_screen = flags6 & 0x08 != 0;
        let mirror = if flags6 & 1 != 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };
        let prg_start = 16 + if flags6 & 4 != 0 { 512 } else { 0 };
        if data.len() < prg_start + prg_len + chr_len {
            return Err("truncated ROM".to_string());
        }
        let prg = data[prg_start..prg_start + prg_len].to_vec();
        let (chr, chr_writable) = if chr_len == 0 {
            (vec![0; 0x2000], true)
        } else {
            (
                data[prg_start + prg_len..prg_start + prg_len + chr_len].to_vec(),
                false,
            )
        };
        let c = Common {
            prg,
            chr,
            chr_writable,
            prg_ram: vec![0; 0x2000],
        };
        let mapper = match mapper_id {
            0 => MapperKind::Nrom(Nrom::new(c, mirror)),
            1 => MapperKind::Mmc1(Mmc1::new(c, Mmc1Regs::new())),
            2 => MapperKind::Uxrom(Uxrom::new(c, mirror)),
            3 => MapperKind::Cnrom(Cnrom::new(c, mirror)),
            4 => MapperKind::Mmc3(Mmc3::new(c, Mmc3Regs::new(mirror))),
            n => return Err(format!("unsupported mapper {}", n)),
        };
        Ok(Cartridge {
            mapper,
            mapper_id,
            four_screen,
        })
    }

    #[inline(always)]
    pub fn cpu_read(&mut self, addr: u16) -> u8 {
        dispatch!(self, m => m.cpu_read(addr))
    }

    pub fn cpu_write(&mut self, addr: u16, value: u8) {
        dispatch!(self, m => m.cpu_write(addr, value))
    }

    #[inline(always)]
    pub fn ppu_read(&mut self, addr: u16) -> u8 {
        dispatch!(self, m => m.ppu_read(addr))
    }

    pub fn ppu_write(&mut self, addr: u16, value: u8) {
        dispatch!(self, m => m.ppu_write(addr, value))
    }

    #[inline(always)]
    pub fn mirroring(&self) -> Mirroring {
        if self.four_screen {
            Mirroring::FourScreen
        } else {
            dispatch!(self, ref m => m.mirroring())
        }
    }

    #[inline]
    pub fn a12(&mut self) {
        if let MapperKind::Mmc3(m) = &mut self.mapper {
            m.a12();
        }
    }

    #[inline]
    pub(crate) fn irq_safe_clocks(&self) -> u32 {
        match &self.mapper {
            MapperKind::Mmc3(m) => {
                if !m.r.irq_enabled || m.r.irq_pending {
                    u32::MAX
                } else if m.r.irq_reload || m.r.irq_counter == 0 {
                    m.r.irq_latch as u32
                } else {
                    m.r.irq_counter as u32 - 1
                }
            }
            _ => u32::MAX,
        }
    }

    #[inline]
    pub fn irq(&mut self) -> bool {
        match &self.mapper {
            MapperKind::Mmc3(m) => m.r.irq_pending,
            _ => false,
        }
    }

    pub fn save(&self, w: &mut Writer) {
        w.u8(self.mapper_id);
        dispatch!(self, ref m => m.save(w))
    }

    pub fn load(&mut self, r: &mut Reader) {
        if r.u8() != self.mapper_id {
            return;
        }
        dispatch!(self, m => m.load(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ines(mapper: u8, prg_banks: u8, chr_banks: u8, flags6_low: u8, trainer: bool) -> Vec<u8> {
        let flags6 = flags6_low | ((mapper & 0x0F) << 4) | if trainer { 4 } else { 0 };
        let flags7 = mapper & 0xF0;
        let mut data = vec![
            b'N', b'E', b'S', 0x1A, prg_banks, chr_banks, flags6, flags7, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        if trainer {
            data.extend(std::iter::repeat_n(0xAA, 512));
        }
        let prg_len = prg_banks as usize * 0x4000;
        let chr_len = chr_banks as usize * 0x2000;
        data.extend((0..prg_len).map(|i| (i % 251) as u8));
        data.extend((0..chr_len).map(|i| ((i * 7) % 251) as u8));
        data
    }

    fn cart(mapper: u8, prg_banks: u8, chr_banks: u8) -> Cartridge {
        Cartridge::from_ines(&ines(mapper, prg_banks, chr_banks, 0, false)).unwrap()
    }

    fn prg_byte(i: usize) -> u8 {
        (i % 251) as u8
    }

    fn chr_byte(i: usize) -> u8 {
        ((i * 7) % 251) as u8
    }

    fn mmc1_serial(c: &mut Cartridge, addr: u16, value: u8) {
        for i in 0..5 {
            c.cpu_write(addr, (value >> i) & 1);
        }
    }

    fn snapshot(c: &mut Cartridge) -> Vec<u8> {
        let mut out = Vec::new();
        let mut a: u32 = 0x6000;
        while a <= 0xFFFF {
            out.push(c.cpu_read(a as u16));
            a += 251;
        }
        for &a in &[
            0x6000u16, 0x7FFF, 0x8000, 0xBFFF, 0xC000, 0xDFFF, 0xE000, 0xFFFF,
        ] {
            out.push(c.cpu_read(a));
        }
        let mut a: u32 = 0;
        while a < 0x2000 {
            out.push(c.ppu_read(a as u16));
            a += 251;
        }
        for &a in &[0u16, 0x03FF, 0x0400, 0x0FFF, 0x1000, 0x1FFF] {
            out.push(c.ppu_read(a));
        }
        out.push(mirroring_to_u8(c.mirroring()));
        out.push(c.irq() as u8);
        out
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(Cartridge::from_ines(&[]).is_err());
        assert!(
            Cartridge::from_ines(b"NES\x00\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00")
                .is_err()
        );
        let mut rom = ines(0, 1, 1, 0, false);
        rom[3] = 0;
        assert!(Cartridge::from_ines(&rom).is_err());
    }

    #[test]
    fn rejects_truncated_and_empty_prg() {
        let rom = ines(0, 1, 1, 0, false);
        assert!(Cartridge::from_ines(&rom[..rom.len() - 1]).is_err());
        assert!(Cartridge::from_ines(&ines(0, 0, 1, 0, false)).is_err());
    }

    #[test]
    fn header_properties_sizes_trainer_mirroring() {
        for prg_banks in [1u8, 2] {
            for chr_banks in [0u8, 1, 2] {
                for mirror_bit in [0u8, 1] {
                    for four_screen in [false, true] {
                        for trainer in [false, true] {
                            let flags6_low = mirror_bit | if four_screen { 8 } else { 0 };
                            let rom = ines(0, prg_banks, chr_banks, flags6_low, trainer);
                            let mut c = Cartridge::from_ines(&rom).unwrap();
                            let expected = if four_screen {
                                Mirroring::FourScreen
                            } else if mirror_bit == 1 {
                                Mirroring::Vertical
                            } else {
                                Mirroring::Horizontal
                            };
                            assert_eq!(c.mirroring(), expected);
                            let prg_len = prg_banks as usize * 0x4000;
                            for addr in [0x8000u16, 0x9234, 0xBFFF, 0xC000, 0xDEAD, 0xFFFF] {
                                let i = (addr as usize - 0x8000) % prg_len;
                                assert_eq!(c.cpu_read(addr), prg_byte(i));
                            }
                            if chr_banks > 0 {
                                for addr in [0u16, 0x1234, 0x1FFF] {
                                    assert_eq!(c.ppu_read(addr), chr_byte(addr as usize));
                                }
                            } else {
                                c.ppu_write(0x123, 0x5A);
                                assert_eq!(c.ppu_read(0x123), 0x5A);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mapper_number_assembly_all_256() {
        for f6 in 0..16u8 {
            for f7 in 0..16u8 {
                let mut rom = ines(0, 1, 1, 0, false);
                rom[6] = f6 << 4;
                rom[7] = f7 << 4;
                let n = (f7 << 4) | f6;
                let result = Cartridge::from_ines(&rom);
                if n <= 4 {
                    assert!(result.is_ok());
                } else {
                    assert_eq!(result.err().unwrap(), format!("unsupported mapper {}", n));
                }
            }
        }
    }

    #[test]
    fn chr_rom_is_read_only() {
        let mut c = cart(0, 1, 1);
        let before = c.ppu_read(0x100);
        c.ppu_write(0x100, before.wrapping_add(1));
        assert_eq!(c.ppu_read(0x100), before);
    }

    #[test]
    fn prg_ram_roundtrip_full_range() {
        for mapper in [0u8, 1, 2, 3, 4] {
            let mut c = cart(mapper, 2, 1);
            for addr in 0x6000u16..=0x7FFF {
                c.cpu_write(addr, (addr ^ (addr >> 8)) as u8);
            }
            for addr in 0x6000u16..=0x7FFF {
                assert_eq!(c.cpu_read(addr), (addr ^ (addr >> 8)) as u8);
            }
        }
    }

    #[test]
    fn cpu_read_below_6000_is_zero() {
        for mapper in [0u8, 1, 2, 3, 4] {
            let mut c = cart(mapper, 2, 1);
            for addr in [0x4020u16, 0x4567, 0x5FFF] {
                c.cpu_write(addr, 0xFF);
                assert_eq!(c.cpu_read(addr), 0);
            }
        }
    }

    #[test]
    fn nrom_prg_mirror_exhaustive() {
        for banks in [1usize, 2] {
            let len = banks * 0x4000;
            for addr in 0x8000u32..=0xFFFF {
                let expected = (addr as usize - 0x8000) % len;
                assert_eq!(nrom_prg_index(addr as u16, len), expected);
            }
        }
    }

    #[test]
    fn uxrom_prg_exhaustive() {
        for banks in [2usize, 4, 8, 16] {
            for bank in 0u8..16 {
                for addr in (0x8000u32..=0xFFFF).step_by(0x100) {
                    let addr = addr as u16;
                    let expected = if addr < 0xC000 {
                        ((bank as usize & 0x0F) % banks) * 0x4000 + (addr as usize - 0x8000)
                    } else {
                        (banks - 1) * 0x4000 + (addr as usize - 0xC000)
                    };
                    assert_eq!(uxrom_prg_index(addr, bank, banks), expected);
                }
            }
        }
        for addr in 0x8000u32..=0xFFFF {
            let addr = addr as u16;
            let got = uxrom_prg_index(addr, 5, 8);
            let expected = if addr < 0xC000 {
                5 * 0x4000 + (addr as usize - 0x8000)
            } else {
                7 * 0x4000 + (addr as usize - 0xC000)
            };
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn uxrom_bank_switch_behavior() {
        let mut c = cart(2, 4, 0);
        for bank in 0u8..16 {
            c.cpu_write(0x8000, bank);
            let sel = (bank & 0x0F) as usize % 4;
            assert_eq!(c.cpu_read(0x8000), prg_byte(sel * 0x4000));
            assert_eq!(c.cpu_read(0xBFFF), prg_byte(sel * 0x4000 + 0x3FFF));
            assert_eq!(c.cpu_read(0xC000), prg_byte(3 * 0x4000));
            assert_eq!(c.cpu_read(0xFFFF), prg_byte(3 * 0x4000 + 0x3FFF));
        }
    }

    #[test]
    fn cnrom_chr_exhaustive() {
        for banks in [1usize, 2, 4] {
            for bank in 0u8..=255 {
                for addr in (0u32..0x2000).step_by(0x40) {
                    let expected = ((bank as usize & 3) % banks) * 0x2000 + addr as usize;
                    assert_eq!(cnrom_chr_index(addr as u16, bank, banks), expected);
                }
            }
        }
        let mut c = cart(3, 1, 4);
        for bank in 0u8..8 {
            c.cpu_write(0x8000, bank);
            let sel = (bank & 3) as usize;
            assert_eq!(c.ppu_read(0), chr_byte(sel * 0x2000));
            assert_eq!(c.ppu_read(0x1FFF), chr_byte(sel * 0x2000 + 0x1FFF));
        }
    }

    fn mmc1_prg_model(addr: u16, mode: u8, bank: u8, banks: usize) -> usize {
        let off = addr as usize - 0x8000;
        let sel = bank as usize % banks;
        match mode {
            0 | 1 => {
                let pair = sel / 2 * 2;
                (pair * 0x4000 + off) % (banks * 0x4000)
            }
            2 => {
                if addr < 0xC000 {
                    off
                } else {
                    sel * 0x4000 + off - 0x4000
                }
            }
            _ => {
                if addr < 0xC000 {
                    sel * 0x4000 + off
                } else {
                    (banks - 1) * 0x4000 + off - 0x4000
                }
            }
        }
    }

    #[test]
    fn mmc1_prg_all_modes_exhaustive() {
        for banks in [2usize, 4, 8, 16] {
            let len = banks * 0x4000;
            for mode in 0u8..4 {
                let control = mode << 2;
                for bank in 0u8..16 {
                    for base in [0x8000u16, 0xBFFF, 0xC000, 0xFFFF, 0x9000, 0xD000] {
                        assert_eq!(
                            mmc1_prg_index(base, control, bank, len),
                            mmc1_prg_model(base, mode, bank, banks),
                            "banks={} mode={} bank={} addr={:#x}",
                            banks,
                            mode,
                            bank,
                            base
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn mmc1_chr_both_modes_exhaustive() {
        for banks in [2usize, 4, 8, 32] {
            let len = banks * 0x1000;
            for chr0 in 0u8..32 {
                for chr1 in 0u8..32 {
                    for addr in (0u32..0x2000).step_by(0x200) {
                        let addr = addr as u16;
                        let eight = {
                            let sel = (chr0 as usize % banks) / 2 * 2;
                            (sel * 0x1000 + addr as usize) % len
                        };
                        let four = if addr < 0x1000 {
                            (chr0 as usize % banks) * 0x1000 + addr as usize
                        } else {
                            (chr1 as usize % banks) * 0x1000 + addr as usize - 0x1000
                        };
                        assert_eq!(mmc1_chr_index(addr, 0x00, chr0, chr1, len), eight);
                        assert_eq!(mmc1_chr_index(addr, 0x10, chr0, chr1, len), four);
                    }
                }
            }
        }
    }

    #[test]
    fn mmc1_shift_register_all_values_all_windows() {
        for window in 0u16..4 {
            let addr = 0x8000 + window * 0x2000;
            for value in 0u8..32 {
                let mut r = Mmc1Regs::new();
                for i in 0..5 {
                    r.write(addr, (value >> i) & 1);
                }
                let (control, chr0, chr1, prg) = match window {
                    0 => (value, 0, 0, 0),
                    1 => (0x0C, value, 0, 0),
                    2 => (0x0C, 0, value, 0),
                    _ => (0x0C, 0, 0, value & 0x0F),
                };
                assert_eq!(r.control, control);
                assert_eq!(r.chr0, chr0);
                assert_eq!(r.chr1, chr1);
                assert_eq!(r.prg, prg);
                assert_eq!(r.count, 0);
                assert_eq!(r.shift, 0);
            }
        }
    }

    #[test]
    fn mmc1_reset_bit_all_values() {
        for value in 0u8..=255 {
            let mut r = Mmc1Regs::new();
            r.control = 0x03;
            r.chr0 = 7;
            r.write(0x8000, 1);
            r.write(0x8000, 1);
            r.write(0x8000, value | 0x80);
            assert_eq!(r.count, 0);
            assert_eq!(r.shift, 0);
            assert_eq!(r.control, 0x0F);
            assert_eq!(r.chr0, 7);
            for i in 0..5 {
                r.write(0xA000, (0x15 >> i) & 1);
            }
            assert_eq!(r.chr0, 0x15);
        }
    }

    #[test]
    fn mmc1_mirroring_via_control() {
        let mut c = cart(1, 2, 1);
        let expect = [
            Mirroring::SingleLower,
            Mirroring::SingleUpper,
            Mirroring::Vertical,
            Mirroring::Horizontal,
        ];
        for (i, &m) in expect.iter().enumerate() {
            mmc1_serial(&mut c, 0x8000, i as u8);
            assert_eq!(c.mirroring(), m);
        }
    }

    fn mmc3_prg_model(addr: u16, mode: bool, r6: u8, r7: u8, banks: usize) -> usize {
        let layout = if mode {
            [
                banks - 2,
                r7 as usize % banks,
                r6 as usize % banks,
                banks - 1,
            ]
        } else {
            [
                r6 as usize % banks,
                r7 as usize % banks,
                banks - 2,
                banks - 1,
            ]
        };
        let window = (addr as usize - 0x8000) / 0x2000;
        layout[window] * 0x2000 + (addr as usize - 0x8000) % 0x2000
    }

    #[test]
    fn mmc3_prg_all_layouts() {
        for banks in [2usize, 4, 8, 16, 32] {
            let len = banks * 0x2000;
            for mode in [false, true] {
                for r6 in 0u8..64 {
                    for r7 in (0u8..64).step_by(7) {
                        for addr in [
                            0x8000u16, 0x9FFF, 0xA000, 0xBFFF, 0xC000, 0xDFFF, 0xE000, 0xFFFF,
                        ] {
                            assert_eq!(
                                mmc3_prg_index(addr, mode, r6, r7, len),
                                mmc3_prg_model(addr, mode, r6, r7, banks)
                            );
                        }
                    }
                }
            }
        }
    }

    fn mmc3_chr_model(addr: u16, inv: bool, regs: &[u8; 8], banks: usize) -> usize {
        let pairs = [
            regs[0] as usize & !1,
            regs[0] as usize | 1,
            regs[1] as usize & !1,
            regs[1] as usize | 1,
        ];
        let singles = [
            regs[2] as usize,
            regs[3] as usize,
            regs[4] as usize,
            regs[5] as usize,
        ];
        let window = addr as usize / 0x400;
        let sel = if inv {
            if window < 4 {
                singles[window]
            } else {
                pairs[window - 4]
            }
        } else if window < 4 {
            pairs[window]
        } else {
            singles[window - 4]
        };
        (sel % banks) * 0x400 + addr as usize % 0x400
    }

    #[test]
    fn mmc3_chr_both_inversions() {
        for banks in [8usize, 16, 32, 64] {
            let len = banks * 0x400;
            for reg in 0usize..6 {
                for value in 0u8..=255 {
                    let mut regs = [2u8, 4, 1, 3, 5, 7, 0, 0];
                    regs[reg] = value;
                    for inv in [false, true] {
                        for window in 0u16..8 {
                            for off in [0u16, 0x3FF] {
                                let addr = window * 0x400 + off;
                                assert_eq!(
                                    mmc3_chr_index(addr, inv, &regs, len),
                                    mmc3_chr_model(addr, inv, &regs, banks)
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn mmc3_bank_select_and_data_writes() {
        let mut c = cart(4, 4, 4);
        for reg in 0u8..8 {
            c.cpu_write(0x8000, reg);
            c.cpu_write(0x8001, reg + 1);
        }
        c.cpu_write(0x8000, 6);
        c.cpu_write(0x8001, 2);
        assert_eq!(c.cpu_read(0x8000), prg_byte(2 * 0x2000));
        c.cpu_write(0x8000, 0x40 | 6);
        c.cpu_write(0x8001, 2);
        assert_eq!(c.cpu_read(0x8000), prg_byte(6 * 0x2000));
        assert_eq!(c.cpu_read(0xC000), prg_byte(2 * 0x2000));
        assert_eq!(c.cpu_read(0xE000), prg_byte(7 * 0x2000));
        c.cpu_write(0xA000, 0);
        assert_eq!(c.mirroring(), Mirroring::Vertical);
        c.cpu_write(0xA000, 1);
        assert_eq!(c.mirroring(), Mirroring::Horizontal);
    }

    #[test]
    fn mmc3_mirroring_ignored_with_four_screen() {
        let mut c = Cartridge::from_ines(&ines(4, 2, 1, 8, false)).unwrap();
        assert_eq!(c.mirroring(), Mirroring::FourScreen);
        c.cpu_write(0xA000, 1);
        assert_eq!(c.mirroring(), Mirroring::FourScreen);
    }

    #[test]
    fn mmc3_irq_schedule_all_latches() {
        for latch in 0u8..=255 {
            let mut c = cart(4, 2, 1);
            c.cpu_write(0xC000, latch);
            c.cpu_write(0xC001, 0);
            c.cpu_write(0xE001, 0);
            let mut counter: u8 = 0;
            let mut reload = true;
            let mut pending = false;
            for _ in 0..600 {
                c.a12();
                if counter == 0 || reload {
                    counter = latch;
                    reload = false;
                } else {
                    counter -= 1;
                }
                if counter == 0 {
                    pending = true;
                }
                assert_eq!(c.irq(), pending, "latch={}", latch);
            }
        }
    }

    #[test]
    fn mmc3_irq_disable_acknowledges_enable_does_not_clear() {
        let mut c = cart(4, 2, 1);
        c.cpu_write(0xC000, 0);
        c.cpu_write(0xC001, 0);
        c.cpu_write(0xE001, 0);
        c.a12();
        assert!(c.irq());
        c.cpu_write(0xE000, 0);
        assert!(!c.irq());
        c.a12();
        assert!(!c.irq());
        c.cpu_write(0xE001, 0);
        assert!(!c.irq());
        c.a12();
        assert!(c.irq());
        c.cpu_write(0xE001, 0);
        assert!(c.irq());
    }

    #[test]
    fn irq_false_and_a12_noop_for_non_mmc3() {
        for mapper in [0u8, 1, 2, 3] {
            let mut c = cart(mapper, 2, 1);
            let before = snapshot(&mut c);
            for _ in 0..10 {
                c.a12();
                assert!(!c.irq());
            }
            assert_eq!(snapshot(&mut c), before);
        }
    }

    fn poke(c: &mut Cartridge, mapper: u8, phase: u8) {
        for addr in (0x6000u16..0x8000).step_by(97) {
            c.cpu_write(addr, addr as u8 ^ phase);
        }
        match mapper {
            1 => {
                mmc1_serial(c, 0x8000, 0x1E ^ (phase & 1));
                mmc1_serial(c, 0xA000, 3 ^ phase);
                mmc1_serial(c, 0xC000, 5 ^ phase);
                mmc1_serial(c, 0xE000, 1 ^ phase);
                c.cpu_write(0x8000, 1);
            }
            2 | 3 => c.cpu_write(0x8000, 2 ^ phase),
            4 => {
                for reg in 0u8..8 {
                    c.cpu_write(0x8000, reg | ((phase & 1) << 6) | ((phase & 2) << 6));
                    c.cpu_write(0x8001, reg.wrapping_mul(3) ^ phase);
                }
                c.cpu_write(0xA000, phase & 1);
                c.cpu_write(0xC000, 5 ^ phase);
                c.cpu_write(0xC001, 0);
                c.cpu_write(0xE001, 0);
                for _ in 0..3 {
                    c.a12();
                }
            }
            _ => {}
        }
    }

    #[test]
    fn save_load_roundtrip_behavior_equivalence() {
        for mapper in [0u8, 1, 2, 3, 4] {
            for chr_banks in [0u8, 4] {
                let rom = ines(mapper, 4, chr_banks, 0, false);
                let mut c = Cartridge::from_ines(&rom).unwrap();
                poke(&mut c, mapper, 0);
                if chr_banks == 0 {
                    for addr in (0u16..0x2000).step_by(89) {
                        c.ppu_write(addr, addr as u8 ^ 0x5A);
                    }
                }
                let expected = snapshot(&mut c);
                let mut w = Writer::new();
                c.save(&mut w);
                poke(&mut c, mapper, 0xFF);
                assert_ne!(snapshot(&mut c), expected);
                let mut r = Reader::new(&w.data);
                c.load(&mut r);
                assert!(r.done());
                assert_eq!(snapshot(&mut c), expected, "mapper={}", mapper);
                let mut fresh = Cartridge::from_ines(&rom).unwrap();
                let mut r = Reader::new(&w.data);
                fresh.load(&mut r);
                assert!(r.done());
                assert_eq!(snapshot(&mut fresh), expected, "mapper={}", mapper);
            }
        }
    }

    #[test]
    fn load_mapper_mismatch_consumes_only_id() {
        let a = cart(0, 2, 1);
        let mut w = Writer::new();
        a.save(&mut w);
        let mut b = cart(2, 2, 1);
        b.cpu_write(0x8000, 1);
        let before = snapshot(&mut b);
        let mut r = Reader::new(&w.data);
        b.load(&mut r);
        assert!(!r.done());
        assert_eq!(snapshot(&mut b), before);
    }

    #[test]
    fn trainer_is_skipped() {
        let rom = ines(0, 1, 1, 0, true);
        assert_eq!(rom.len(), 16 + 512 + 0x4000 + 0x2000);
        let mut c = Cartridge::from_ines(&rom).unwrap();
        assert_eq!(c.cpu_read(0x8000), prg_byte(0));
        assert_eq!(c.ppu_read(0), chr_byte(0));
        assert_eq!(c.cpu_read(0xBFFF), prg_byte(0x3FFF));
    }
}
