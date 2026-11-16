use crate::cartridge::{Cartridge, Mirroring};
use crate::state::{Reader, Writer};

const MASTER_PALETTE: [u32; 64] = [
    0x666666, 0x002A88, 0x1412A7, 0x3B00A4, 0x5C007E, 0x6E0040, 0x6C0600, 0x561D00, 0x333500,
    0x0B4800, 0x005200, 0x004F08, 0x00404D, 0x000000, 0x000000, 0x000000, 0xADADAD, 0x155FD9,
    0x4240FF, 0x7527FE, 0xA01ACC, 0xB71E7B, 0xB53120, 0x994E00, 0x6B6D00, 0x388700, 0x0C9300,
    0x008F32, 0x007C8D, 0x000000, 0x000000, 0x000000, 0xFFFEFF, 0x64B0FF, 0x9290FF, 0xC676FF,
    0xF36AFF, 0xFE6ECC, 0xFE8170, 0xEA9E22, 0xBCBE00, 0x88D800, 0x5CE430, 0x45E082, 0x48CDDE,
    0x4F4F4F, 0x000000, 0x000000, 0xFFFEFF, 0xC0DFFF, 0xD3D2FF, 0xE8C8FF, 0xFBC2FF, 0xFEC4EA,
    0xFECCC5, 0xF7D8A5, 0xE4E594, 0xCFEF96, 0xBDF4AB, 0xB3F3CC, 0xB5EBF2, 0xB8B8B8, 0x000000,
    0x000000,
];

const NMI_DELAY: u8 = 2;
const A12_FILTER: u64 = 10;
const A12_IRQ_DELAY: u8 = 3;

const fn rgb_to_pixel(rgb: u32) -> u32 {
    let r = (rgb >> 16) & 0xFF;
    let g = (rgb >> 8) & 0xFF;
    let b = rgb & 0xFF;
    0xFF00_0000 | (b << 16) | (g << 8) | r
}

const PIXEL_PALETTE: [u32; 64] = {
    let mut out = [0u32; 64];
    let mut i = 0;
    while i < 64 {
        out[i] = rgb_to_pixel(MASTER_PALETTE[i]);
        i += 1;
    }
    out
};

const MIRROR_BANK: [[usize; 4]; 5] = [
    [0, 0, 1, 1],
    [0, 1, 0, 1],
    [0, 0, 0, 0],
    [1, 1, 1, 1],
    [0, 1, 0, 1],
];

#[inline(always)]
pub(crate) fn mirror(mode: Mirroring, addr: u16) -> usize {
    let addr = (addr as usize) & 0x0FFF;
    let table = (addr >> 10) & 3;
    let bank = MIRROR_BANK[mode as usize][table];
    bank * 0x400 + (addr & 0x3FF)
}

pub(crate) fn palette_index(addr: u16) -> usize {
    let i = (addr as usize) & 0x1F;
    if i >= 0x10 && i & 3 == 0 {
        i - 0x10
    } else {
        i
    }
}

pub(crate) fn increment_coarse_x(v: u16) -> u16 {
    if v & 0x001F == 0x001F {
        (v & !0x001F) ^ 0x0400
    } else {
        v + 1
    }
}

pub(crate) fn increment_y(v: u16) -> u16 {
    if v & 0x7000 != 0x7000 {
        return v + 0x1000;
    }
    let mut v = v & !0x7000;
    let coarse_y = (v >> 5) & 0x1F;
    let new_y = match coarse_y {
        29 => {
            v ^= 0x0800;
            0
        }
        31 => 0,
        y => y + 1,
    };
    (v & !0x03E0) | (new_y << 5)
}

pub(crate) fn write_ctrl_t(t: u16, value: u8) -> u16 {
    (t & !0x0C00) | (((value as u16) & 3) << 10)
}

pub(crate) fn write_scroll(t: u16, x: u8, w: bool, value: u8) -> (u16, u8, bool) {
    let d = value as u16;
    if w {
        let t = (t & 0x8C1F) | ((d & 7) << 12) | ((d & 0xF8) << 2);
        (t, x, false)
    } else {
        let t = (t & !0x001F) | (d >> 3);
        (t, value & 7, true)
    }
}

pub(crate) fn write_addr(t: u16, v: u16, w: bool, value: u8) -> (u16, u16, bool) {
    let d = value as u16;
    if w {
        let t = (t & 0x7F00) | d;
        (t, t, false)
    } else {
        let t = (t & 0x00FF) | ((d & 0x3F) << 8);
        (t, v, true)
    }
}

pub(crate) fn addr_increment(ctrl: u8) -> u16 {
    if ctrl & 0x04 != 0 {
        32
    } else {
        1
    }
}

pub(crate) fn sprite_in_range(sprite_y: u8, scanline: u16, tall: bool) -> bool {
    let height = 8 << (tall as u16);
    scanline.wrapping_sub(sprite_y as u16) < height
}

const SPREAD: [u32; 256] = {
    let mut out = [0u32; 256];
    let mut b = 0;
    while b < 256 {
        let mut v = 0u32;
        let mut j = 0;
        while j < 8 {
            v |= (((b >> j) & 1) as u32) << (4 * j);
            j += 1;
        }
        out[b] = v;
        b += 1;
    }
    out
};

pub(crate) fn pack_pipe(s0: u16, s1: u16, s2: u16, s3: u16) -> u64 {
    let mut pipe = 0u64;
    let mut i = 0;
    while i < 16 {
        let nib = (((s0 >> i) & 1) as u64)
            | ((((s1 >> i) & 1) as u64) << 1)
            | ((((s2 >> i) & 1) as u64) << 2)
            | ((((s3 >> i) & 1) as u64) << 3);
        pipe |= nib << (4 * i);
        i += 1;
    }
    pipe
}

pub(crate) fn unpack_pipe(pipe: u64) -> (u16, u16, u16, u16) {
    let mut s = [0u16; 4];
    let mut i = 0;
    while i < 16 {
        let nib = (pipe >> (4 * i)) & 0xF;
        let mut k = 0;
        while k < 4 {
            s[k] |= (((nib >> k) & 1) as u16) << i;
            k += 1;
        }
        i += 1;
    }
    (s[0], s[1], s[2], s[3])
}

pub(crate) fn bg_pattern_addr(table: bool, tile: u8, fine_y: u16) -> u16 {
    ((table as u16) << 12) | ((tile as u16) << 4) | (fine_y & 7)
}

pub(crate) fn tall_sprite_addr(tile: u8, row: u16) -> u16 {
    let bank = ((tile & 1) as u16) << 12;
    let index = ((tile & 0xFE) as u16) + (row >> 3);
    bank | (index << 4) | (row & 7)
}

pub struct Ppu {
    pub frame: Vec<u32>,
    pub frame_ready: bool,
    pub nmi: bool,
    ctrl: u8,
    mask: u8,
    status: u8,
    oam_addr: u8,
    v: u16,
    t: u16,
    x: u8,
    w: bool,
    data_buffer: u8,
    open_bus: u8,
    vram: [u8; 2048],
    palette: [u8; 32],
    oam: [u8; 256],
    sec_oam: [u8; 32],
    scanline: u16,
    dot: u16,
    odd_frame: bool,
    skip_latch: bool,
    master: u64,
    suppress_vbl: bool,
    nmi_delay: u8,
    a12_state: bool,
    a12_fall: u64,
    a12_pending: u8,
    nt_byte: u8,
    at_byte: u8,
    pt_lo: u8,
    pt_hi: u8,
    bg_pipe: u64,
    oam_latch: u8,
    eval_n: u8,
    eval_m: u8,
    eval_sec: u8,
    eval_state: u8,
    eval_copy: u8,
    eval_first: u8,
    spr0_next: bool,
    spr0_cur: bool,
    spr_count: u8,
    spr_addr: u16,
    spr_x: [u8; 8],
    spr_attr: [u8; 8],
    spr_lo: [u8; 8],
    spr_hi: [u8; 8],
    pal_pix: [u32; 32],
}

impl Default for Ppu {
    fn default() -> Self {
        Ppu::new()
    }
}

impl Ppu {
    pub fn new() -> Self {
        Ppu {
            frame: vec![0xFF00_0000; 256 * 240],
            frame_ready: false,
            nmi: false,
            ctrl: 0,
            mask: 0,
            status: 0,
            oam_addr: 0,
            v: 0,
            t: 0,
            x: 0,
            w: false,
            data_buffer: 0,
            open_bus: 0,
            vram: [0; 2048],
            palette: [0; 32],
            oam: [0; 256],
            sec_oam: [0xFF; 32],
            scanline: 261,
            dot: 0,
            odd_frame: false,
            skip_latch: false,
            master: 0,
            suppress_vbl: false,
            nmi_delay: 0,
            a12_state: false,
            a12_fall: 0,
            a12_pending: 0,
            nt_byte: 0,
            at_byte: 0,
            pt_lo: 0,
            pt_hi: 0,
            bg_pipe: 0,
            oam_latch: 0,
            eval_n: 0,
            eval_m: 0,
            eval_sec: 0,
            eval_state: 3,
            eval_copy: 0,
            eval_first: 0,
            spr0_next: false,
            spr0_cur: false,
            spr_count: 0,
            spr_addr: 0,
            spr_x: [0; 8],
            spr_attr: [0; 8],
            spr_lo: [0; 8],
            spr_hi: [0; 8],
            pal_pix: [PIXEL_PALETTE[0]; 32],
        }
    }

    fn rebuild_pal_pix(&mut self) {
        let grey = if self.mask & 0x01 != 0 { 0x30 } else { 0x3F };
        for i in 0..32 {
            self.pal_pix[i] = PIXEL_PALETTE[(self.palette[i] & grey) as usize & 0x3F];
        }
    }

    #[inline(always)]
    fn set_bus(&mut self, cart: &mut Cartridge, addr: u16) {
        if addr & 0x1000 != 0 {
            if !self.a12_state && self.master.wrapping_sub(self.a12_fall) >= A12_FILTER {
                if self.a12_pending > 0 {
                    cart.a12();
                }
                self.a12_pending = A12_IRQ_DELAY;
            }
            self.a12_state = true;
        } else if self.a12_state {
            self.a12_state = false;
            self.a12_fall = self.master;
        }
    }

    #[inline(always)]
    fn mem_read(&mut self, cart: &mut Cartridge, addr: u16) -> u8 {
        let addr = addr & 0x3FFF;
        self.set_bus(cart, addr);
        match addr {
            0x0000..=0x1FFF => cart.ppu_read(addr),
            0x2000..=0x3EFF => self.vram[mirror(cart.mirroring(), addr)],
            _ => self.palette[palette_index(addr)],
        }
    }

    fn mem_write(&mut self, cart: &mut Cartridge, addr: u16, value: u8) {
        let addr = addr & 0x3FFF;
        self.set_bus(cart, addr);
        match addr {
            0x0000..=0x1FFF => cart.ppu_write(addr, value),
            0x2000..=0x3EFF => self.vram[mirror(cart.mirroring(), addr)] = value,
            _ => {
                self.palette[palette_index(addr)] = value;
                self.rebuild_pal_pix();
            }
        }
    }

    fn nmi_level(&self) -> bool {
        self.status & 0x80 != 0 && self.ctrl & 0x80 != 0
    }

    fn start_nmi(&mut self) {
        if self.nmi_delay == 0 {
            self.nmi_delay = NMI_DELAY;
        }
    }

    fn shift_registers(&mut self) {
        self.bg_pipe <<= 4;
    }

    fn reload_shifters(&mut self) {
        let at = self.at_byte as u64;
        let fresh = SPREAD[self.pt_lo as usize] as u64
            | ((SPREAD[self.pt_hi as usize] as u64) << 1)
            | ((at & 1) * 0x4444_4444)
            | (((at >> 1) & 1) * 0x8888_8888);
        self.bg_pipe = (self.bg_pipe & 0xFFFF_FFFF_0000_0000) | fresh;
    }

    #[inline(always)]
    fn fetch_step(&mut self, cart: &mut Cartridge, phase: u16) {
        match phase {
            0 => {
                self.reload_shifters();
                self.nt_byte = self.mem_read(cart, 0x2000 | (self.v & 0x0FFF));
            }
            2 => {
                let addr =
                    0x23C0 | (self.v & 0x0C00) | ((self.v >> 4) & 0x38) | ((self.v >> 2) & 0x07);
                let mut at = self.mem_read(cart, addr);
                if self.v & 0x0040 != 0 {
                    at >>= 4;
                }
                if self.v & 0x0002 != 0 {
                    at >>= 2;
                }
                self.at_byte = at & 3;
            }
            3 => {
                let addr = bg_pattern_addr(self.ctrl & 0x10 != 0, self.nt_byte, (self.v >> 12) & 7);
                self.set_bus(cart, addr);
            }
            4 => {
                let addr = bg_pattern_addr(self.ctrl & 0x10 != 0, self.nt_byte, (self.v >> 12) & 7);
                self.pt_lo = self.mem_read(cart, addr);
            }
            5 => {
                let addr = bg_pattern_addr(self.ctrl & 0x10 != 0, self.nt_byte, (self.v >> 12) & 7);
                self.set_bus(cart, addr + 8);
            }
            6 => {
                let addr = bg_pattern_addr(self.ctrl & 0x10 != 0, self.nt_byte, (self.v >> 12) & 7);
                self.pt_hi = self.mem_read(cart, addr + 8);
            }
            7 => {
                self.v = increment_coarse_x(self.v);
            }
            _ => {}
        }
    }

    fn eval_step(&mut self, line: u16) {
        let tall = self.ctrl & 0x20 != 0;
        match self.eval_state {
            0 => {
                let y = self.oam_latch;
                if self.eval_sec < 32 {
                    self.sec_oam[self.eval_sec as usize] = y;
                }
                if sprite_in_range(y, line, tall) {
                    if self.eval_n == self.eval_first {
                        self.spr0_next = true;
                    }
                    self.eval_sec += 1;
                    self.eval_m = 1;
                    self.eval_state = 1;
                } else {
                    self.eval_n = self.eval_n.wrapping_add(1) & 63;
                    if self.eval_n == 0 {
                        self.eval_state = 3;
                    }
                }
            }
            1 => {
                self.sec_oam[self.eval_sec as usize] = self.oam_latch;
                self.eval_sec += 1;
                self.eval_m += 1;
                if self.eval_m == 4 {
                    self.eval_m = 0;
                    self.eval_n = self.eval_n.wrapping_add(1) & 63;
                    if self.eval_n == 0 {
                        self.eval_state = 3;
                    } else if self.eval_sec == 32 {
                        self.eval_state = 2;
                    } else {
                        self.eval_state = 0;
                    }
                }
            }
            2 => {
                if sprite_in_range(self.oam_latch, line, tall) {
                    self.status |= 0x20;
                    self.eval_copy = 3;
                    self.eval_m = (self.eval_m + 1) & 3;
                    self.eval_state = 4;
                } else {
                    self.eval_m = (self.eval_m + 1) & 3;
                    self.eval_n = self.eval_n.wrapping_add(1) & 63;
                    if self.eval_n == 0 {
                        self.eval_state = 3;
                    }
                }
            }
            4 => {
                self.eval_copy -= 1;
                self.eval_m += 1;
                if self.eval_m == 4 {
                    self.eval_m = 0;
                    self.eval_n = self.eval_n.wrapping_add(1) & 63;
                }
                if self.eval_copy == 0 {
                    self.eval_state = 3;
                }
            }
            _ => {
                self.eval_n = self.eval_n.wrapping_add(1) & 63;
            }
        }
        self.oam_addr = self.eval_n.wrapping_mul(4).wrapping_add(self.eval_m);
    }

    #[inline(always)]
    fn sprite_fetch(&mut self, cart: &mut Cartridge, line: u16, dot: u16) {
        self.oam_addr = 0;
        let g = ((dot - 257) / 8) as usize;
        let phase = (dot - 257) % 8;
        match phase {
            0 => {
                if g == 0 {
                    if line == 261 {
                        self.spr_count = 0;
                        self.spr0_cur = false;
                    } else {
                        self.spr_count = self.eval_sec / 4;
                        self.spr0_cur = self.spr0_next;
                    }
                }
                self.set_bus(cart, 0x2000 | (self.v & 0x0FFF));
            }
            2 => {
                self.set_bus(cart, 0x2000 | (self.v & 0x0FFF));
                self.spr_attr[g] = self.sec_oam[g * 4 + 2];
            }
            3 => {
                self.spr_x[g] = self.sec_oam[g * 4 + 3];
                let tall = self.ctrl & 0x20 != 0;
                let empty = line == 261 || g >= self.spr_count as usize;
                let (tile, row) = if empty {
                    (0xFFu8, 0u16)
                } else {
                    let y = self.sec_oam[g * 4] as u16;
                    let tile = self.sec_oam[g * 4 + 1];
                    let height: u16 = if tall { 16 } else { 8 };
                    let mut row = line.wrapping_sub(y) & (height - 1);
                    if self.spr_attr[g] & 0x80 != 0 {
                        row = height - 1 - row;
                    }
                    (tile, row)
                };
                self.spr_addr = if tall {
                    tall_sprite_addr(tile, row)
                } else {
                    bg_pattern_addr(self.ctrl & 0x08 != 0, tile, row)
                };
                self.set_bus(cart, self.spr_addr);
            }
            4 => {
                let empty = line == 261 || g >= self.spr_count as usize;
                let mut lo = self.mem_read(cart, self.spr_addr);
                if empty {
                    lo = 0;
                } else if self.spr_attr[g] & 0x40 != 0 {
                    lo = lo.reverse_bits();
                }
                self.spr_lo[g] = lo;
            }
            5 => {
                self.set_bus(cart, self.spr_addr + 8);
            }
            6 => {
                let empty = line == 261 || g >= self.spr_count as usize;
                let mut hi = self.mem_read(cart, self.spr_addr + 8);
                if empty {
                    hi = 0;
                } else if self.spr_attr[g] & 0x40 != 0 {
                    hi = hi.reverse_bits();
                }
                self.spr_hi[g] = hi;
            }
            _ => {}
        }
    }

    #[inline(always)]
    fn render_pixel(&mut self, dot: u16, line: u16) {
        let px = (dot - 1) as usize & 0xFF;
        let line = line as usize;
        let mut bg_pixel = 0u8;
        let mut bg_pal = 0u8;
        if self.mask & 0x08 != 0 && (px >= 8 || self.mask & 0x02 != 0) {
            let nib = ((self.bg_pipe >> (((15 - self.x as u32) & 15) * 4)) & 0xF) as u8;
            bg_pixel = nib & 3;
            bg_pal = nib >> 2;
        }
        let mut spr_pixel = 0u8;
        let mut spr_pal = 0u8;
        let mut spr_behind = false;
        let mut spr_is_zero = false;
        if self.mask & 0x10 != 0 && (px >= 8 || self.mask & 0x04 != 0) {
            let count = (self.spr_count as usize).min(8);
            for n in 0..count {
                let sx = self.spr_x[n] as usize;
                if px.wrapping_sub(sx) >= 8 {
                    continue;
                }
                let shift = 7 - (px - sx);
                let p0 = (self.spr_lo[n] >> shift) & 1;
                let p1 = (self.spr_hi[n] >> shift) & 1;
                let p = (p1 << 1) | p0;
                if p != 0 {
                    spr_pixel = p;
                    spr_pal = (self.spr_attr[n] & 3) + 4;
                    spr_behind = self.spr_attr[n] & 0x20 != 0;
                    spr_is_zero = self.spr0_cur && n == 0;
                    break;
                }
            }
        }
        if spr_is_zero && bg_pixel != 0 && spr_pixel != 0 && px != 255 {
            self.status |= 0x40;
        }
        let pal_addr = if bg_pixel == 0 && spr_pixel == 0 {
            0
        } else if bg_pixel == 0 {
            (spr_pal << 2) | spr_pixel
        } else if spr_pixel == 0 || spr_behind {
            (bg_pal << 2) | bg_pixel
        } else {
            (spr_pal << 2) | spr_pixel
        };
        self.frame[line * 256 + px] = self.pal_pix[pal_addr as usize & 0x1F];
    }

    pub fn tick(&mut self, cart: &mut Cartridge) {
        self.tick_inner(cart);
    }

    pub(crate) fn defer_dots(&self, cart: &Cartridge) -> u32 {
        if self.nmi_delay != 0 || self.a12_pending != 0 {
            return 0;
        }
        let line = self.scanline as u32;
        let dot = self.dot as u32;
        let frame = 341 * 262;
        let cur = line * 341 + dot;
        let mut d = ((241 * 341 + 1) + frame - cur) % frame;
        let safe = cart.irq_safe_clocks();
        if safe != u32::MAX && self.mask & 0x18 != 0 {
            let fetch_line = line < 240 || line == 261;
            let df = if self.ctrl & 0x10 != 0 {
                if fetch_line {
                    return 0;
                }
                (341 - dot) + (260 - line) * 341
            } else if fetch_line && (257..=320).contains(&dot) {
                if safe < 16 {
                    return 0;
                }
                let next = (341 - dot) + 257 + if line == 239 { 21 * 341 } else { 0 };
                next + ((safe - 16) / 8).min(262) * 341
            } else {
                let first = if !fetch_line {
                    (341 - dot) + (260 - line) * 341 + 257
                } else if dot < 257 {
                    257 - dot
                } else {
                    (341 - dot) + 257 + if line == 239 { 21 * 341 } else { 0 }
                };
                if safe < 8 {
                    first
                } else {
                    first + ((safe - 8) / 8).min(262) * 341
                }
            };
            d = d.min(df);
        }
        d.saturating_sub(8)
    }

    pub(crate) fn catch_up(&mut self, cart: &mut Cartridge, mut dots: u32) {
        while dots > 0 {
            let line = self.scanline;
            dots = if line < 240 {
                self.run_visible_span(cart, line, dots)
            } else if line == 261 {
                self.run_prerender_span(cart, dots)
            } else {
                self.run_idle_span(cart, line, dots)
            };
        }
    }

    fn run_idle_span(&mut self, cart: &mut Cartridge, line: u16, dots: u32) -> u32 {
        if self.a12_pending | self.nmi_delay != 0 || (line == 241 && self.dot <= 1) {
            self.tick_inner(cart);
            return dots - 1;
        }
        let span = (341 - self.dot) as u32;
        let n = span.min(dots);
        self.master = self.master.wrapping_add(n as u64);
        if line == 241 {
            self.suppress_vbl = false;
        }
        self.dot += n as u16;
        if self.dot == 341 {
            self.dot = 0;
            self.scanline += 1;
        }
        dots - n
    }

    fn run_prerender_span(&mut self, cart: &mut Cartridge, mut dots: u32) -> u32 {
        let rendering = self.mask & 0x18 != 0;
        while dots > 0 && self.scanline == 261 {
            if !rendering
                && self.a12_pending | self.nmi_delay == 0
                && self.dot != 1
                && self.dot != 338
            {
                let d = self.dot;
                let stop = if d < 1 {
                    1
                } else if d < 338 {
                    338
                } else {
                    341
                };
                let n = ((stop - d) as u32).min(dots);
                self.master = self.master.wrapping_add(n as u64);
                self.dot += n as u16;
                dots -= n;
            } else {
                self.master = self.master.wrapping_add(1);
                self.step_pipelines(cart);
                self.prerender_tick(cart);
                self.dot += 1;
                dots -= 1;
            }
            if self.dot == 341 {
                self.dot = 0;
                self.scanline = 0;
                self.odd_frame = !self.odd_frame;
            }
        }
        dots
    }

    fn run_visible_span(&mut self, cart: &mut Cartridge, line: u16, mut dots: u32) -> u32 {
        if self.dot == 0 {
            self.tick_inner(cart);
            dots -= 1;
            if dots == 0 {
                return 0;
            }
        }
        let rendering = self.mask & 0x18 != 0;
        if !rendering {
            if self.a12_pending | self.nmi_delay != 0 {
                self.tick_inner(cart);
                return dots - 1;
            }
            let d = self.dot;
            let end = ((d as u32 + dots).min(341)) as u16;
            if d <= 256 {
                let pend = end.min(257);
                let color = self.pal_pix[0];
                let row = line as usize * 256;
                for px in (d - 1)..(pend - 1) {
                    self.frame[row + px as usize] = color;
                }
            }
            let n = (end - d) as u32;
            self.master = self.master.wrapping_add(n as u64);
            self.dot = end;
            if end == 341 {
                self.dot = 0;
                self.scanline += 1;
            }
            return dots - n;
        }
        let end = ((self.dot as u32 + dots).min(341)) as u16;
        let n = (end - self.dot) as u32;
        let mut d = self.dot;
        if n >= 8 {
            while d < end {
                if d & 7 == 1 && d < 256 && d + 8 <= end {
                    self.tile_group(cart, line, d);
                    d += 8;
                } else if d & 7 == 1 && (257..=313).contains(&d) && d + 8 <= end {
                    self.sprite_group(cart, line, d);
                    d += 8;
                } else if (d == 321 || d == 329) && d + 8 <= end {
                    self.fetch_group(cart, d == 321, d + 7);
                    d += 8;
                } else {
                    self.master = self.master.wrapping_add(1);
                    self.step_pipelines(cart);
                    self.visible_dot(cart, line, d, true);
                    d += 1;
                }
            }
        } else {
            while d < end {
                self.master = self.master.wrapping_add(1);
                self.step_pipelines(cart);
                self.visible_dot(cart, line, d, true);
                d += 1;
            }
        }
        self.dot = end;
        if end == 341 {
            self.dot = 0;
            self.scanline += 1;
        }
        dots - n
    }

    fn sprite_group(&mut self, cart: &mut Cartridge, line: u16, d0: u16) {
        let g = ((d0 - 257) >> 3) as usize;
        self.pump(cart);
        if d0 == 257 {
            self.shift_registers();
            self.reload_shifters();
            self.v = (self.v & !0x041F) | (self.t & 0x041F);
            self.spr_count = self.eval_sec / 4;
            self.spr0_cur = self.spr0_next;
        }
        self.set_bus(cart, 0x2000 | (self.v & 0x0FFF));
        self.pump(cart);
        self.pump(cart);
        self.set_bus(cart, 0x2000 | (self.v & 0x0FFF));
        self.spr_attr[g] = self.sec_oam[g * 4 + 2];
        self.pump(cart);
        self.spr_x[g] = self.sec_oam[g * 4 + 3];
        let tall = self.ctrl & 0x20 != 0;
        let empty = g >= self.spr_count as usize;
        let (tile, row) = if empty {
            (0xFFu8, 0u16)
        } else {
            let y = self.sec_oam[g * 4] as u16;
            let tile = self.sec_oam[g * 4 + 1];
            let height: u16 = if tall { 16 } else { 8 };
            let mut row = line.wrapping_sub(y) & (height - 1);
            if self.spr_attr[g] & 0x80 != 0 {
                row = height - 1 - row;
            }
            (tile, row)
        };
        self.spr_addr = if tall {
            tall_sprite_addr(tile, row)
        } else {
            bg_pattern_addr(self.ctrl & 0x08 != 0, tile, row)
        };
        self.set_bus(cart, self.spr_addr);
        self.pump(cart);
        let mut lo = self.mem_read(cart, self.spr_addr);
        if empty {
            lo = 0;
        } else if self.spr_attr[g] & 0x40 != 0 {
            lo = lo.reverse_bits();
        }
        self.spr_lo[g] = lo;
        self.pump(cart);
        self.set_bus(cart, self.spr_addr + 8);
        self.pump(cart);
        let mut hi = self.mem_read(cart, self.spr_addr + 8);
        if empty {
            hi = 0;
        } else if self.spr_attr[g] & 0x40 != 0 {
            hi = hi.reverse_bits();
        }
        self.spr_hi[g] = hi;
        self.pump(cart);
        self.oam_addr = 0;
    }

    #[inline(always)]
    fn pump(&mut self, cart: &mut Cartridge) {
        self.master = self.master.wrapping_add(1);
        self.step_pipelines(cart);
    }

    #[inline(always)]
    fn fetch_group(&mut self, cart: &mut Cartridge, no_shift: bool, last_dot: u16) -> u64 {
        self.pump(cart);
        let keep = if no_shift {
            self.bg_pipe
        } else {
            self.bg_pipe << 4
        };
        let at = self.at_byte as u64;
        let fresh = SPREAD[self.pt_lo as usize] as u64
            | ((SPREAD[self.pt_hi as usize] as u64) << 1)
            | ((at & 1) * 0x4444_4444)
            | (((at >> 1) & 1) * 0x8888_8888);
        let p0 = (keep & 0xFFFF_FFFF_0000_0000) | fresh;
        self.nt_byte = self.mem_read(cart, 0x2000 | (self.v & 0x0FFF));
        self.pump(cart);
        self.pump(cart);
        let at_addr = 0x23C0 | (self.v & 0x0C00) | ((self.v >> 4) & 0x38) | ((self.v >> 2) & 0x07);
        let mut atb = self.mem_read(cart, at_addr);
        if self.v & 0x0040 != 0 {
            atb >>= 4;
        }
        if self.v & 0x0002 != 0 {
            atb >>= 2;
        }
        self.at_byte = atb & 3;
        self.pump(cart);
        let pt_addr = bg_pattern_addr(self.ctrl & 0x10 != 0, self.nt_byte, (self.v >> 12) & 7);
        self.set_bus(cart, pt_addr);
        self.pump(cart);
        self.pt_lo = self.mem_read(cart, pt_addr);
        self.pump(cart);
        self.set_bus(cart, pt_addr + 8);
        self.pump(cart);
        self.pt_hi = self.mem_read(cart, pt_addr + 8);
        self.pump(cart);
        self.v = increment_coarse_x(self.v);
        if last_dot == 256 {
            self.v = increment_y(self.v);
        }
        self.bg_pipe = p0 << 28;
        p0
    }

    fn tile_group(&mut self, cart: &mut Cartridge, line: u16, d0: u16) {
        let p0 = self.fetch_group(cart, d0 == 1, d0 + 7);
        if d0 + 7 <= 64 {
            self.oam_latch = 0xFF;
            let base = (d0 >> 1) as usize & 31;
            self.sec_oam[base] = 0xFF;
            self.sec_oam[base + 1] = 0xFF;
            self.sec_oam[base + 2] = 0xFF;
            self.sec_oam[base + 3] = 0xFF;
            if d0 + 7 == 64 {
                self.eval_n = self.oam_addr >> 2;
                self.eval_m = self.oam_addr & 3;
                self.eval_first = self.eval_n;
                self.eval_sec = 0;
                self.eval_state = 0;
                self.eval_copy = 0;
                self.spr0_next = false;
            }
        } else if self.eval_state == 3 {
            let m = self.eval_m;
            let n3 = self.eval_n.wrapping_add(3) & 63;
            self.oam_latch = self.oam[(n3 as usize) * 4 + m as usize];
            self.eval_n = self.eval_n.wrapping_add(4) & 63;
            self.oam_addr = self.eval_n.wrapping_mul(4).wrapping_add(m);
        } else {
            for _ in 0..4 {
                self.oam_latch = self.oam[(self.eval_n as usize) * 4 + self.eval_m as usize];
                self.eval_step(line);
            }
        }
        let bg_show = self.mask & 0x08 != 0 && (d0 != 1 || self.mask & 0x02 != 0);
        let spr_show = self.mask & 0x10 != 0 && (d0 != 1 || self.mask & 0x04 != 0);
        let count = if spr_show {
            (self.spr_count as usize).min(8)
        } else {
            0
        };
        let base = (d0 - 1) as usize & 0xFF;
        let row = line as usize * 256;
        let shift_base = (15 - self.x as u32) * 4;
        for i in 0..8usize {
            let px = base + i;
            let mut bg_pixel = 0u8;
            let mut bg_pal = 0u8;
            if bg_show {
                let nib = ((p0 >> (shift_base - 4 * i as u32)) & 0xF) as u8;
                bg_pixel = nib & 3;
                bg_pal = nib >> 2;
            }
            let mut spr_pixel = 0u8;
            let mut spr_pal = 0u8;
            let mut spr_behind = false;
            let mut spr_is_zero = false;
            for n in 0..count {
                let sx = self.spr_x[n] as usize;
                if px.wrapping_sub(sx) >= 8 {
                    continue;
                }
                let shift = 7 - (px - sx);
                let p0b = (self.spr_lo[n] >> shift) & 1;
                let p1b = (self.spr_hi[n] >> shift) & 1;
                let p = (p1b << 1) | p0b;
                if p != 0 {
                    spr_pixel = p;
                    spr_pal = (self.spr_attr[n] & 3) + 4;
                    spr_behind = self.spr_attr[n] & 0x20 != 0;
                    spr_is_zero = self.spr0_cur && n == 0;
                    break;
                }
            }
            if spr_is_zero && bg_pixel != 0 && spr_pixel != 0 && px != 255 {
                self.status |= 0x40;
            }
            let pal_addr = if bg_pixel == 0 && spr_pixel == 0 {
                0
            } else if bg_pixel == 0 {
                (spr_pal << 2) | spr_pixel
            } else if spr_pixel == 0 || spr_behind {
                (bg_pal << 2) | bg_pixel
            } else {
                (spr_pal << 2) | spr_pixel
            };
            self.frame[row + px] = self.pal_pix[pal_addr as usize & 0x1F];
        }
    }

    #[inline(always)]
    fn step_pipelines(&mut self, cart: &mut Cartridge) {
        if self.a12_pending | self.nmi_delay != 0 {
            if self.a12_pending > 0 {
                self.a12_pending -= 1;
                if self.a12_pending == 0 {
                    cart.a12();
                }
            }
            if self.nmi_delay > 0 {
                self.nmi_delay -= 1;
                if self.nmi_delay == 0 && self.nmi_level() {
                    self.nmi = true;
                }
            }
        }
    }

    #[inline(always)]
    fn tick_inner(&mut self, cart: &mut Cartridge) {
        self.master = self.master.wrapping_add(1);
        self.step_pipelines(cart);
        let line = self.scanline;
        if line < 240 {
            self.visible_tick(cart, line);
        } else if line == 261 {
            self.prerender_tick(cart);
        } else if line == 241 {
            let dot = self.dot;
            if dot == 1 {
                self.frame_ready = true;
                if !self.suppress_vbl {
                    self.status |= 0x80;
                    if self.ctrl & 0x80 != 0 {
                        self.start_nmi();
                    }
                }
            }
            if dot >= 1 {
                self.suppress_vbl = false;
            }
        }
        self.dot += 1;
        if self.dot == 341 {
            self.dot = 0;
            self.scanline += 1;
            if self.scanline == 262 {
                self.scanline = 0;
                self.odd_frame = !self.odd_frame;
            }
        }
    }

    #[inline(always)]
    fn visible_tick(&mut self, cart: &mut Cartridge, line: u16) {
        let mut dot = self.dot;
        if dot == 0 {
            if line == 0 && self.skip_latch {
                self.skip_latch = false;
                self.dot = 1;
                dot = 1;
            } else {
                return;
            }
        }
        let rendering = self.mask & 0x18 != 0;
        self.visible_dot(cart, line, dot, rendering);
    }

    #[inline(always)]
    fn visible_dot(&mut self, cart: &mut Cartridge, line: u16, dot: u16, rendering: bool) {
        if dot <= 256 {
            if rendering {
                if dot >= 2 {
                    self.shift_registers();
                }
                self.fetch_step(cart, (dot - 1) & 7);
                if dot == 256 {
                    self.v = increment_y(self.v);
                }
                if dot <= 64 {
                    if dot & 1 == 1 {
                        self.oam_latch = 0xFF;
                    } else {
                        self.sec_oam[((dot - 1) >> 1) as usize & 31] = 0xFF;
                    }
                    if dot == 64 {
                        self.eval_n = self.oam_addr >> 2;
                        self.eval_m = self.oam_addr & 3;
                        self.eval_first = self.eval_n;
                        self.eval_sec = 0;
                        self.eval_state = 0;
                        self.eval_copy = 0;
                        self.spr0_next = false;
                    }
                } else if dot & 1 == 1 {
                    self.oam_latch = self.oam[(self.eval_n as usize) * 4 + self.eval_m as usize];
                } else {
                    self.eval_step(line);
                }
            }
            self.render_pixel(dot, line);
        } else if dot <= 320 {
            if rendering {
                if dot == 257 {
                    self.shift_registers();
                    self.reload_shifters();
                    self.v = (self.v & !0x041F) | (self.t & 0x041F);
                }
                self.sprite_fetch(cart, line, dot);
            }
        } else if rendering {
            if dot <= 336 {
                if dot >= 322 {
                    self.shift_registers();
                }
                self.fetch_step(cart, (dot - 1) & 7);
            } else if dot == 337 {
                self.shift_registers();
            }
        }
    }

    #[inline(always)]
    fn prerender_tick(&mut self, cart: &mut Cartridge) {
        let dot = self.dot;
        if dot == 1 {
            self.status &= 0x1F;
            self.nmi_delay = 0;
        }
        if dot == 338 {
            self.skip_latch = self.odd_frame && self.mask & 0x18 != 0;
        }
        if self.mask & 0x18 == 0 {
            return;
        }
        if dot == 0 {
        } else if dot <= 256 {
            if dot >= 2 {
                self.shift_registers();
            }
            self.fetch_step(cart, (dot - 1) & 7);
            if dot == 256 {
                self.v = increment_y(self.v);
            }
        } else if dot <= 320 {
            if dot == 257 {
                self.shift_registers();
                self.reload_shifters();
                self.v = (self.v & !0x041F) | (self.t & 0x041F);
            }
            if (280..=304).contains(&dot) {
                self.v = (self.v & !0x7BE0) | (self.t & 0x7BE0);
            }
            self.sprite_fetch(cart, 261, dot);
        } else if dot <= 336 {
            if dot >= 322 {
                self.shift_registers();
            }
            self.fetch_step(cart, (dot - 1) & 7);
        } else if dot == 337 {
            self.shift_registers();
        }
    }

    pub fn cpu_read(&mut self, reg: u16, cart: &mut Cartridge) -> u8 {
        match reg {
            2 => {
                let mut value = (self.status & 0xE0) | (self.open_bus & 0x1F);
                if self.scanline == 241 && self.dot == 1 {
                    value &= !0x80;
                    self.suppress_vbl = true;
                }
                self.status &= !0x80;
                self.nmi_delay = 0;
                self.w = false;
                value
            }
            4 => {
                let rendering = self.mask & 0x18 != 0;
                if rendering && self.scanline < 240 {
                    let dot = self.dot;
                    if (1..=64).contains(&dot) {
                        0xFF
                    } else if (65..=256).contains(&dot) {
                        self.oam_latch
                    } else if (257..=320).contains(&dot) {
                        let g = ((dot - 257) / 8) as usize;
                        let ph = ((dot - 257) % 8) as usize;
                        self.sec_oam[g * 4 + ph.min(3)]
                    } else {
                        self.sec_oam[0]
                    }
                } else {
                    self.oam[self.oam_addr as usize]
                }
            }
            7 => {
                let addr = self.v & 0x3FFF;
                let value = if addr >= 0x3F00 {
                    let grey = if self.mask & 1 != 0 { 0x30 } else { 0x3F };
                    self.set_bus(cart, addr);
                    self.data_buffer = self.vram[mirror(cart.mirroring(), addr)];
                    self.palette[palette_index(addr)] & grey
                } else {
                    let buffered = self.data_buffer;
                    self.data_buffer = self.mem_read(cart, addr);
                    buffered
                };
                self.v = (self.v + addr_increment(self.ctrl)) & 0x7FFF;
                if self.mask & 0x18 == 0 || self.scanline >= 240 && self.scanline != 261 {
                    self.set_bus(cart, self.v & 0x3FFF);
                }
                value
            }
            _ => self.open_bus,
        }
    }

    pub fn cpu_write(&mut self, reg: u16, value: u8, cart: &mut Cartridge) {
        self.open_bus = value;
        match reg {
            0 => {
                let old_level = self.nmi_level();
                self.ctrl = value;
                self.t = write_ctrl_t(self.t, value);
                let new_level = self.nmi_level();
                if new_level && !old_level {
                    self.start_nmi();
                }
                if !new_level {
                    self.nmi_delay = 0;
                }
            }
            1 => {
                self.mask = value;
                self.rebuild_pal_pix();
            }
            3 => self.oam_addr = value,
            4 => {
                self.oam_store(self.oam_addr, value);
                self.oam_addr = self.oam_addr.wrapping_add(1);
            }
            5 => {
                let (t, x, w) = write_scroll(self.t, self.x, self.w, value);
                self.t = t;
                self.x = x;
                self.w = w;
            }
            6 => {
                let (t, v, w) = write_addr(self.t, self.v, self.w, value);
                self.t = t;
                self.v = v;
                self.w = w;
                if !w && (self.mask & 0x18 == 0 || self.scanline >= 240 && self.scanline != 261) {
                    self.set_bus(cart, self.v & 0x3FFF);
                }
            }
            7 => {
                let addr = self.v & 0x3FFF;
                self.mem_write(cart, addr, value);
                self.v = (self.v + addr_increment(self.ctrl)) & 0x7FFF;
                if self.mask & 0x18 == 0 || self.scanline >= 240 && self.scanline != 261 {
                    self.set_bus(cart, self.v & 0x3FFF);
                }
            }
            _ => {}
        }
    }

    fn oam_store(&mut self, index: u8, value: u8) {
        let i = index as usize;
        self.oam[i] = if i & 3 == 2 { value & 0xE3 } else { value };
    }

    pub fn oam_dma_write(&mut self, index: u8, value: u8) {
        self.oam_store(self.oam_addr.wrapping_add(index), value);
    }

    pub fn save(&self, w: &mut Writer) {
        w.u8(self.ctrl);
        w.u8(self.mask);
        w.u8(self.status);
        w.u8(self.oam_addr);
        w.u16(self.v);
        w.u16(self.t);
        w.u8(self.x);
        w.bool(self.w);
        w.u8(self.data_buffer);
        w.u8(self.open_bus);
        w.bytes(&self.vram);
        w.bytes(&self.palette);
        w.bytes(&self.oam);
        w.bytes(&self.sec_oam);
        w.u16(self.scanline);
        w.u16(self.dot);
        w.bool(self.odd_frame);
        w.bool(self.skip_latch);
        w.bool(self.nmi);
        w.u64(self.master);
        w.bool(self.suppress_vbl);
        w.u8(self.nmi_delay);
        w.bool(self.a12_state);
        w.u64(self.a12_fall);
        w.u8(self.a12_pending);
        w.u8(self.nt_byte);
        w.u8(self.at_byte);
        w.u8(self.pt_lo);
        w.u8(self.pt_hi);
        let (s0, s1, s2, s3) = unpack_pipe(self.bg_pipe);
        w.u16(s0);
        w.u16(s1);
        w.u16(s2);
        w.u16(s3);
        w.u8(self.oam_latch);
        w.u8(self.eval_n);
        w.u8(self.eval_m);
        w.u8(self.eval_sec);
        w.u8(self.eval_state);
        w.u8(self.eval_copy);
        w.u8(self.eval_first);
        w.bool(self.spr0_next);
        w.bool(self.spr0_cur);
        w.u8(self.spr_count);
        w.u16(self.spr_addr);
        w.bytes(&self.spr_x);
        w.bytes(&self.spr_attr);
        w.bytes(&self.spr_lo);
        w.bytes(&self.spr_hi);
    }

    pub fn load(&mut self, r: &mut Reader) {
        self.ctrl = r.u8();
        self.mask = r.u8();
        self.status = r.u8();
        self.oam_addr = r.u8();
        self.v = r.u16();
        self.t = r.u16();
        self.x = r.u8();
        self.w = r.bool();
        self.data_buffer = r.u8();
        self.open_bus = r.u8();
        r.bytes(&mut self.vram);
        r.bytes(&mut self.palette);
        r.bytes(&mut self.oam);
        r.bytes(&mut self.sec_oam);
        self.scanline = r.u16();
        self.dot = r.u16();
        self.odd_frame = r.bool();
        self.skip_latch = r.bool();
        self.nmi = r.bool();
        self.master = r.u64();
        self.suppress_vbl = r.bool();
        self.nmi_delay = r.u8();
        self.a12_state = r.bool();
        self.a12_fall = r.u64();
        self.a12_pending = r.u8();
        self.nt_byte = r.u8();
        self.at_byte = r.u8();
        self.pt_lo = r.u8();
        self.pt_hi = r.u8();
        self.bg_pipe = pack_pipe(r.u16(), r.u16(), r.u16(), r.u16());
        self.oam_latch = r.u8();
        self.eval_n = r.u8();
        self.eval_m = r.u8();
        self.eval_sec = r.u8();
        self.eval_state = r.u8();
        self.eval_copy = r.u8();
        self.eval_first = r.u8();
        self.spr0_next = r.bool();
        self.spr0_cur = r.bool();
        self.spr_count = r.u8();
        self.spr_addr = r.u16();
        r.bytes(&mut self.spr_x);
        r.bytes(&mut self.spr_attr);
        r.bytes(&mut self.spr_lo);
        r.bytes(&mut self.spr_hi);
        self.rebuild_pal_pix();
        self.frame_ready = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cart() -> Cartridge {
        let mut bytes = vec![0x4E, 0x45, 0x53, 0x1A, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bytes.extend_from_slice(&[0u8; 16384]);
        Cartridge::from_ines(&bytes).unwrap()
    }

    #[test]
    fn mirror_exhaustive() {
        type MirrorCase = (&'static str, fn() -> Mirroring, [usize; 4]);
        let cases: [MirrorCase; 5] = [
            ("horizontal", || Mirroring::Horizontal, [0, 0, 1, 1]),
            ("vertical", || Mirroring::Vertical, [0, 1, 0, 1]),
            ("single_lower", || Mirroring::SingleLower, [0, 0, 0, 0]),
            ("single_upper", || Mirroring::SingleUpper, [1, 1, 1, 1]),
            ("four_screen", || Mirroring::FourScreen, [0, 1, 0, 1]),
        ];
        for (name, make, banks) in cases {
            for addr in 0x2000u16..0x3000 {
                let nt = ((addr - 0x2000) / 0x400) as usize;
                let expected = banks[nt] * 0x400 + ((addr as usize - 0x2000) % 0x400);
                let got = mirror(make(), addr);
                assert!(got < 2048);
                assert_eq!(got, expected, "mode {} addr {:04X}", name, addr);
            }
        }
    }

    #[test]
    fn palette_index_exhaustive() {
        for addr in 0x3F00u16..=0x3FFF {
            let mut expected = (addr as usize) % 32;
            if expected.is_multiple_of(4) && expected >= 16 {
                expected -= 16;
            }
            let got = palette_index(addr);
            assert!(got < 32);
            assert_eq!(got, expected, "addr {:04X}", addr);
        }
    }

    #[test]
    fn coarse_x_increment_exhaustive() {
        for v in 0u16..0x8000 {
            let got = increment_coarse_x(v);
            let coarse = v & 0x1F;
            if coarse == 31 {
                assert_eq!(got & 0x1F, 0);
                assert_eq!(got & 0x0400, (v & 0x0400) ^ 0x0400);
                assert_eq!(got & !0x041F, v & !0x041F);
            } else {
                assert_eq!(got, v + 1);
            }
            assert!(got < 0x8000);
        }
    }

    #[test]
    fn y_increment_exhaustive() {
        for v in 0u16..0x8000 {
            let got = increment_y(v);
            let fine_y = (v >> 12) & 7;
            let coarse_y = (v >> 5) & 0x1F;
            if fine_y < 7 {
                assert_eq!(got, v + 0x1000);
            } else {
                assert_eq!((got >> 12) & 7, 0);
                assert_eq!(got & 0x041F, v & 0x041F);
                match coarse_y {
                    29 => {
                        assert_eq!((got >> 5) & 0x1F, 0);
                        assert_eq!(got & 0x0800, (v & 0x0800) ^ 0x0800);
                    }
                    31 => {
                        assert_eq!((got >> 5) & 0x1F, 0);
                        assert_eq!(got & 0x0800, v & 0x0800);
                    }
                    y => {
                        assert_eq!((got >> 5) & 0x1F, y + 1);
                        assert_eq!(got & 0x0800, v & 0x0800);
                    }
                }
            }
            assert!(got < 0x8000);
        }
    }

    #[test]
    fn ctrl_write_updates_nametable_bits() {
        for t0 in [0u16, 0x7FFF, 0x5A5A, 0x0C00] {
            for value in 0u16..=255 {
                let got = write_ctrl_t(t0, value as u8);
                assert_eq!(got & !0x0C00, t0 & !0x0C00);
                assert_eq!((got >> 10) & 3, value & 3);
            }
        }
    }

    #[test]
    fn scroll_write_first_and_second() {
        for t0 in [0u16, 0x7FFF, 0x2D6B] {
            for value in 0u16..=255 {
                let (t, x, w) = write_scroll(t0, 5, false, value as u8);
                assert!(w);
                assert_eq!(x as u16, value & 7);
                assert_eq!(t & 0x1F, value >> 3);
                assert_eq!(t & !0x001F, t0 & !0x001F);

                let (t, x, w) = write_scroll(t0, 5, true, value as u8);
                assert!(!w);
                assert_eq!(x, 5);
                assert_eq!((t >> 12) & 7, value & 7);
                assert_eq!((t >> 5) & 0x1F, value >> 3);
                assert_eq!(t & 0x0C1F, t0 & 0x0C1F);
                assert_eq!(t & 0x8000, t0 & 0x8000);
            }
        }
    }

    #[test]
    fn addr_write_first_and_second() {
        for t0 in [0u16, 0x7FFF, 0x33CC] {
            for value in 0u16..=255 {
                let (t, v, w) = write_addr(t0, 0x1234, false, value as u8);
                assert!(w);
                assert_eq!(v, 0x1234);
                assert_eq!(t & 0x00FF, t0 & 0x00FF);
                assert_eq!(t >> 8, value & 0x3F);

                let (t, v, w) = write_addr(t0, 0x1234, true, value as u8);
                assert!(!w);
                assert_eq!(t & 0x00FF, value);
                assert_eq!(t & 0x7F00, t0 & 0x7F00);
                assert_eq!(v, t);
            }
        }
    }

    #[test]
    fn data_port_increment_modes() {
        let mut cart = test_cart();
        for start in (0u16..0x4000).step_by(97) {
            for &(ctrl, step) in &[(0u8, 1u16), (0x04, 32)] {
                let mut ppu = Ppu::new();
                ppu.ctrl = ctrl;
                ppu.v = start;
                ppu.cpu_read(7, &mut cart);
                assert_eq!(ppu.v, (start + step) & 0x7FFF);
                ppu.v = start;
                ppu.cpu_write(7, 0, &mut cart);
                assert_eq!(ppu.v, (start + step) & 0x7FFF);
            }
        }
        assert_eq!(addr_increment(0xFB), 1);
        assert_eq!(addr_increment(0x04), 32);
    }

    #[test]
    fn sprite_in_range_exhaustive() {
        for sprite_y in 0u16..=255 {
            for scanline in 0u16..=239 {
                let expect8 = scanline >= sprite_y && scanline < sprite_y + 8;
                let expect16 = scanline >= sprite_y && scanline < sprite_y + 16;
                assert_eq!(sprite_in_range(sprite_y as u8, scanline, false), expect8);
                assert_eq!(sprite_in_range(sprite_y as u8, scanline, true), expect16);
            }
        }
    }

    #[test]
    fn bg_pattern_addr_exhaustive() {
        for table in [false, true] {
            for tile in 0u16..=255 {
                for fine_y in 0u16..8 {
                    let expected = (table as u16) * 0x1000 + tile * 16 + fine_y;
                    assert_eq!(bg_pattern_addr(table, tile as u8, fine_y), expected);
                }
            }
        }
    }

    #[test]
    fn tall_sprite_addr_exhaustive() {
        for tile in 0u16..=255 {
            for row in 0u16..16 {
                let bank = (tile & 1) * 0x1000;
                let base = tile & 0xFE;
                let expected = if row < 8 {
                    bank + base * 16 + row
                } else {
                    bank + (base + 1) * 16 + (row - 8)
                };
                assert_eq!(tall_sprite_addr(tile as u8, row), expected);
            }
        }
    }

    fn set_addr(ppu: &mut Ppu, cart: &mut Cartridge, addr: u16) {
        ppu.cpu_read(2, cart);
        ppu.cpu_write(6, (addr >> 8) as u8, cart);
        ppu.cpu_write(6, (addr & 0xFF) as u8, cart);
    }

    #[test]
    fn buffered_reads_lag_except_palette() {
        let mut cart = test_cart();
        let mut ppu = Ppu::new();
        for addr in (0u16..0x3F00).step_by(0x101) {
            set_addr(&mut ppu, &mut cart, addr);
            ppu.cpu_write(7, (addr & 0xFF) as u8 ^ 0xA5, &mut cart);
        }
        for addr in (0u16..0x3F00).step_by(0x101) {
            let expected = (addr & 0xFF) as u8 ^ 0xA5;
            set_addr(&mut ppu, &mut cart, addr);
            ppu.cpu_read(7, &mut cart);
            set_addr(&mut ppu, &mut cart, addr);
            let second = ppu.cpu_read(7, &mut cart);
            assert_eq!(second, expected, "addr {:04X}", addr);
        }
        set_addr(&mut ppu, &mut cart, 0x2020);
        ppu.cpu_read(7, &mut cart);
        set_addr(&mut ppu, &mut cart, 0x0000);
        let stale = ppu.cpu_read(7, &mut cart);
        assert_eq!(stale, 0x20 ^ 0xA5);

        for i in 0u16..32 {
            set_addr(&mut ppu, &mut cart, 0x3F00 + i);
            ppu.cpu_write(7, 0x20 + palette_index(0x3F00 + i) as u8, &mut cart);
        }
        for i in 0u16..256 {
            let addr = 0x3F00 + i;
            set_addr(&mut ppu, &mut cart, addr);
            let direct = ppu.cpu_read(7, &mut cart);
            let expected = 0x20 + palette_index(addr) as u8;
            assert_eq!(direct, expected, "palette addr {:04X}", addr);
        }

        set_addr(&mut ppu, &mut cart, 0x2F00);
        ppu.cpu_write(7, 0x5A, &mut cart);
        set_addr(&mut ppu, &mut cart, 0x3F00);
        ppu.cpu_read(7, &mut cart);
        set_addr(&mut ppu, &mut cart, 0x0100);
        assert_eq!(ppu.cpu_read(7, &mut cart), 0x5A);
    }

    #[test]
    fn status_read_clears_vblank_and_latch() {
        let mut cart = test_cart();
        let mut ppu = Ppu::new();
        ppu.status = 0xE0;
        ppu.w = true;
        ppu.open_bus = 0x1F;
        let value = ppu.cpu_read(2, &mut cart);
        assert_eq!(value, 0xFF);
        assert_eq!(ppu.status & 0x80, 0);
        assert_eq!(ppu.status & 0x60, 0x60);
        assert!(!ppu.w);
        ppu.open_bus = 0;
        assert_eq!(ppu.cpu_read(2, &mut cart), 0x60);
    }

    #[test]
    fn vblank_nmi_and_frame_timing() {
        let mut cart = test_cart();
        let mut ppu = Ppu::new();
        ppu.cpu_write(0, 0x80, &mut cart);
        assert!(!ppu.nmi);
        let mut set_at = None;
        for i in 0u32..(341 * 262 * 2) {
            ppu.tick(&mut cart);
            if ppu.frame_ready && set_at.is_none() {
                set_at = Some(i);
                assert!(!ppu.nmi);
                assert_eq!(ppu.status & 0x80, 0x80);
                assert_eq!(ppu.scanline, 241);
                assert_eq!(ppu.dot, 2);
                for _ in 0..NMI_DELAY {
                    ppu.tick(&mut cart);
                }
                assert!(ppu.nmi);
                ppu.nmi = false;
            }
            ppu.frame_ready = false;
        }
        assert!(set_at.is_some());
        ppu.nmi = false;
        ppu.status = 0x80;
        ppu.ctrl = 0;
        ppu.cpu_write(0, 0x80, &mut cart);
        assert!(!ppu.nmi);
        for _ in 0..NMI_DELAY {
            ppu.tick(&mut cart);
        }
        assert!(ppu.nmi);
    }

    #[test]
    fn status_read_one_dot_before_set_suppresses_flag() {
        let mut cart = test_cart();
        let mut ppu = Ppu::new();
        ppu.cpu_write(0, 0x80, &mut cart);
        while !(ppu.scanline == 241 && ppu.dot == 1) {
            ppu.tick(&mut cart);
        }
        let v = ppu.cpu_read(2, &mut cart);
        assert_eq!(v & 0x80, 0);
        for _ in 0..30 {
            ppu.tick(&mut cart);
        }
        assert_eq!(ppu.status & 0x80, 0);
        assert!(!ppu.nmi);
    }

    #[test]
    fn status_read_just_after_set_suppresses_nmi() {
        let mut cart = test_cart();
        let mut ppu = Ppu::new();
        ppu.cpu_write(0, 0x80, &mut cart);
        while !(ppu.scanline == 241 && ppu.dot == 2) {
            ppu.tick(&mut cart);
        }
        let v = ppu.cpu_read(2, &mut cart);
        assert_eq!(v & 0x80, 0x80);
        for _ in 0..30 {
            ppu.tick(&mut cart);
        }
        assert!(!ppu.nmi);
        assert_eq!(ppu.status & 0x80, 0);
    }

    fn mmc3_cart(latch: u8) -> Cartridge {
        let mut rom = vec![
            b'N', b'E', b'S', 0x1A, 1, 0, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        rom.extend(std::iter::repeat_n(0, 16384));
        let mut cart = Cartridge::from_ines(&rom).unwrap();
        cart.cpu_write(0xC000, latch);
        cart.cpu_write(0xC001, 0);
        cart.cpu_write(0xE001, 0);
        cart
    }

    fn run_frame_count_via_mmc3(latch: u8, mask: u8) -> bool {
        let mut cart = mmc3_cart(latch);
        let mut ppu = Ppu::new();
        ppu.scanline = 0;
        ppu.dot = 0;
        ppu.cpu_write(0, 0x08, &mut cart);
        ppu.cpu_write(1, mask, &mut cart);
        for _ in 0..341 * 262 {
            ppu.tick(&mut cart);
        }
        cart.irq()
    }

    #[test]
    fn a12_sprite_fetches_clock_mmc3_once_per_rendered_line() {
        assert!(!run_frame_count_via_mmc3(1, 0x00));
        assert!(run_frame_count_via_mmc3(240, 0x18));
        assert!(!run_frame_count_via_mmc3(241, 0x18));
    }

    #[test]
    fn a12_filter_blocks_fast_toggles() {
        for gap in 1u64..30 {
            let mut cart = mmc3_cart(0);
            let mut ppu = Ppu::new();
            let mut clocks = 0;
            for i in 0..20 {
                ppu.master = 1000 + i * 2 * gap;
                ppu.set_bus(&mut cart, 0x1000);
                ppu.master = 1000 + i * 2 * gap + gap;
                ppu.set_bus(&mut cart, 0x0000);
                let m = ppu.master;
                for _ in 0..A12_IRQ_DELAY + 1 {
                    ppu.tick(&mut cart);
                }
                ppu.master = m;
                if cart.irq() {
                    clocks += 1;
                    cart.cpu_write(0xE000, 0);
                    cart.cpu_write(0xE001, 0);
                }
            }
            if gap >= A12_FILTER {
                assert_eq!(clocks, 20, "gap {}", gap);
            } else {
                assert_eq!(clocks, 1, "gap {}", gap);
            }
        }
    }

    #[test]
    fn a12_rises_via_addr_and_data_port_clock_mmc3() {
        let mut cart = mmc3_cart(0);
        let mut ppu = Ppu::new();
        for i in 0u64..10 {
            ppu.master = 1000 + i * 100;
            ppu.cpu_write(6, 0x10, &mut cart);
            ppu.cpu_write(6, 0x00, &mut cart);
            let m = ppu.master;
            for _ in 0..A12_IRQ_DELAY + 1 {
                ppu.tick(&mut cart);
            }
            ppu.master = m;
            assert!(cart.irq(), "iteration {}", i);
            cart.cpu_write(0xE000, 0);
            cart.cpu_write(0xE001, 0);
            ppu.master = 1000 + i * 100 + 50;
            ppu.cpu_write(6, 0x00, &mut cart);
            ppu.cpu_write(6, 0x00, &mut cart);
            assert!(!cart.irq());
        }
    }

    #[test]
    fn odd_frame_skips_one_dot_when_rendering() {
        let mut cart = test_cart();
        let mut ppu = Ppu::new();
        ppu.scanline = 0;
        ppu.dot = 0;
        ppu.odd_frame = false;
        let full: u32 = 341 * 262;
        let measure = |ppu: &mut Ppu, cart: &mut Cartridge| {
            let mut count = 0;
            loop {
                ppu.tick(cart);
                count += 1;
                if ppu.scanline == 0 && ppu.dot == 0 {
                    break;
                }
            }
            count
        };
        ppu.cpu_write(1, 0x00, &mut cart);
        for _ in 0..6 {
            assert_eq!(measure(&mut ppu, &mut cart), full);
        }
        ppu.cpu_write(1, 0x08, &mut cart);
        let lengths: Vec<u32> = (0..6).map(|_| measure(&mut ppu, &mut cart)).collect();
        let shorts: Vec<usize> = (0..6).filter(|&i| lengths[i] == full - 1).collect();
        for (i, &len) in lengths.iter().enumerate() {
            assert!(len == full || len == full - 1, "frame {} len {}", i, len);
        }
        assert!(shorts.len() >= 2);
        for pair in shorts.windows(2) {
            assert_eq!(pair[1] - pair[0], 2);
        }
        ppu.cpu_write(1, 0x00, &mut cart);
        for _ in 0..2 {
            measure(&mut ppu, &mut cart);
        }
        for _ in 0..6 {
            assert_eq!(measure(&mut ppu, &mut cart), full);
        }
    }

    #[test]
    fn oam_access_via_registers_and_dma() {
        let mut cart = test_cart();
        let mut ppu = Ppu::new();
        for i in 0u16..=255 {
            ppu.oam_dma_write(i as u8, i as u8 ^ 0x3C);
        }
        for i in 0u16..=255 {
            ppu.cpu_write(3, i as u8, &mut cart);
            let mut expected = i as u8 ^ 0x3C;
            if i & 3 == 2 {
                expected &= 0xE3;
            }
            assert_eq!(ppu.cpu_read(4, &mut cart), expected);
        }
        ppu.cpu_write(3, 10, &mut cart);
        ppu.cpu_write(4, 0x77, &mut cart);
        assert_eq!(ppu.oam_addr, 11);
        assert_eq!(ppu.oam[10], 0x77 & 0xE3);
        ppu.cpu_write(3, 0x20, &mut cart);
        for i in 0u16..=255 {
            ppu.oam_dma_write(i as u8, i as u8);
        }
        assert_eq!(ppu.oam_addr, 0x20);
        for i in 0u16..=255 {
            let slot = (0x20 + i) as u8;
            let mut expected = i as u8;
            if slot & 3 == 2 {
                expected &= 0xE3;
            }
            assert_eq!(ppu.oam[slot as usize], expected, "slot {}", slot);
        }
    }

    fn eval_model(oam: &[u8; 256], line: u16, tall: bool) -> (Vec<usize>, bool) {
        let height = if tall { 16 } else { 8 };
        let mut found = Vec::new();
        let mut overflow = false;
        let mut n = 0usize;
        let mut m = 0usize;
        while n < 64 {
            let y = oam[n * 4 + m] as u16;
            if found.len() < 8 {
                if line >= y && line < y + height {
                    found.push(n);
                }
                n += 1;
            } else {
                if line >= y && line < y + height {
                    overflow = true;
                    break;
                }
                n += 1;
                m = (m + 1) & 3;
            }
        }
        (found, overflow)
    }

    fn run_line_eval(ppu: &mut Ppu, cart: &mut Cartridge, line: u16) {
        ppu.scanline = line;
        ppu.dot = 0;
        ppu.eval_state = 3;
        while !(ppu.scanline == line && ppu.dot == 257) {
            ppu.tick(cart);
        }
    }

    #[test]
    fn sprite_evaluation_matches_model_including_overflow_bug() {
        let mut cart = test_cart();
        let seeds = [1u32, 7, 1234, 99999, 424242, 777];
        for (case, seed) in seeds.into_iter().enumerate() {
            let mut state = seed;
            let mut rng = || {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                (state >> 24) as u8
            };
            for tall in [false, true] {
                let mut ppu = Ppu::new();
                for i in 0..256 {
                    ppu.oam[i] = if i % 4 == 0 { rng() % 64 } else { rng() };
                }
                ppu.cpu_write(1, 0x18, &mut cart);
                ppu.cpu_write(0, if tall { 0x20 } else { 0 }, &mut cart);
                for line in [0u16, 10, 30, 63, 100] {
                    ppu.oam_addr = 0;
                    run_line_eval(&mut ppu, &mut cart, line);
                    let (found, overflow) = eval_model(&ppu.oam, line, tall);
                    assert_eq!(
                        ppu.eval_sec as usize / 4,
                        found.len(),
                        "case {} tall {} line {}",
                        case,
                        tall,
                        line
                    );
                    for (slot, &n) in found.iter().enumerate() {
                        assert_eq!(
                            &ppu.sec_oam[slot * 4..slot * 4 + 4],
                            &ppu.oam[n * 4..n * 4 + 4]
                        );
                    }
                    assert_eq!(ppu.status & 0x20 != 0, overflow);
                    assert_eq!(ppu.spr0_next, found.first() == Some(&0));
                    ppu.status = 0;
                }
            }
        }
    }

    #[test]
    fn save_load_roundtrip() {
        let mut cart = test_cart();
        let mut ppu = Ppu::new();
        ppu.cpu_write(0, 0x90, &mut cart);
        ppu.cpu_write(1, 0x1E, &mut cart);
        ppu.cpu_write(5, 0x7D, &mut cart);
        ppu.cpu_write(5, 0x5E, &mut cart);
        for _ in 0..12345 {
            ppu.tick(&mut cart);
        }
        for i in 0..64 {
            ppu.oam_dma_write(i * 4, i);
        }
        let mut w = Writer::new();
        ppu.save(&mut w);
        let mut other = Ppu::new();
        let mut r = Reader::new(&w.data);
        other.load(&mut r);
        assert!(r.done());
        assert!(!other.frame_ready);
        assert_eq!(other.v, ppu.v);
        assert_eq!(other.t, ppu.t);
        assert_eq!(other.x, ppu.x);
        assert_eq!(other.w, ppu.w);
        assert_eq!(other.scanline, ppu.scanline);
        assert_eq!(other.dot, ppu.dot);
        assert_eq!(other.oam, ppu.oam);
        assert_eq!(other.vram, ppu.vram);
        assert_eq!(other.palette, ppu.palette);
        let mut w2 = Writer::new();
        other.save(&mut w2);
        assert_eq!(w.data, w2.data);
    }

    #[test]
    fn pixel_format_matches_spec() {
        for (i, &rgb) in MASTER_PALETTE.iter().enumerate() {
            let px = rgb_to_pixel(rgb);
            let r = (rgb >> 16) & 0xFF;
            let g = (rgb >> 8) & 0xFF;
            let b = rgb & 0xFF;
            assert_eq!(px, 0xFF00_0000 | (b << 16) | (g << 8) | r, "entry {}", i);
        }
        assert_eq!(MASTER_PALETTE.len(), 64);
    }
}
