use crate::apu::Apu;
use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::controller::Controller;
use crate::cpu::Cpu;
use crate::ppu::Ppu;
use crate::state::{Reader, Writer};

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 240;

pub struct Machine {
    pub ppu: Ppu,
    pub apu: Apu,
    pub cart: Cartridge,
    pub ram: [u8; 2048],
    pub pads: [Controller; 2],
    pending_nmi: bool,
    cycles: u64,
    ppu_debt: u32,
    defer: u32,
}

impl Machine {
    fn latch_nmi(&mut self) {
        if self.ppu.nmi {
            self.ppu.nmi = false;
            self.pending_nmi = true;
        }
    }

    fn dmc_read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[addr as usize & 0x07FF],
            _ => self.cart.cpu_read(addr),
        }
    }

    #[inline(always)]
    fn tick(&mut self) {
        self.cycles = self.cycles.wrapping_add(1);
        self.apu.tick();
        self.ppu_debt += 3;
        if self.apu.dmc_hungry() {
            self.dmc_stall();
        }
    }

    #[cold]
    fn dmc_stall(&mut self) {
        if let Some(addr) = self.apu.dmc_request() {
            let v = self.dmc_read(addr);
            self.apu.dmc_supply(v);
            for _ in 0..4 {
                self.cycles = self.cycles.wrapping_add(1);
                self.apu.tick();
                self.ppu_debt += 3;
            }
        }
    }

    #[inline]
    fn catch_up(&mut self) {
        let dots = self.ppu_debt;
        if dots != 0 {
            self.ppu_debt = 0;
            self.ppu.catch_up(&mut self.cart, dots);
            self.rearm();
        }
    }

    #[inline]
    fn rearm(&mut self) {
        self.defer = self.ppu.defer_dots(&self.cart);
    }

    fn tick_sync(&mut self) {
        self.tick();
        self.catch_up();
    }

    #[inline]
    fn read_mem(&mut self, addr: u16) -> u8 {
        if addr >= 0x4020 {
            return self.cart.cpu_read(addr);
        }
        match addr {
            0x0000..=0x1FFF => self.ram[addr as usize & 0x07FF],
            0x2000..=0x3FFF => {
                self.catch_up();
                let v = self.ppu.cpu_read(addr & 7, &mut self.cart);
                self.rearm();
                v
            }
            0x4015 => self.apu.read_status(),
            0x4016 => self.pads[0].read() | 0x40,
            0x4017 => self.pads[1].read() | 0x40,
            _ => 0,
        }
    }

    fn write_mem(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram[addr as usize & 0x07FF] = value,
            0x2000..=0x3FFF => {
                self.catch_up();
                self.ppu.cpu_write(addr & 7, value, &mut self.cart);
                self.rearm();
            }
            0x4014 => self.oam_dma(value),
            0x4016 => {
                self.pads[0].write_strobe(value);
                self.pads[1].write_strobe(value);
            }
            0x4000..=0x4017 => self.apu.write(addr, value),
            0x4018..=0x401F => {}
            _ => {
                self.catch_up();
                self.cart.cpu_write(addr, value);
                self.rearm();
            }
        }
    }

    fn oam_dma(&mut self, page: u8) {
        let base = (page as u16) << 8;
        self.tick_sync();
        if self.cycles & 1 == 1 {
            self.tick_sync();
        }
        for i in 0..256u16 {
            self.tick_sync();
            let v = self.read_mem(base + i);
            self.tick_sync();
            self.ppu.oam_dma_write(i as u8, v);
        }
    }
}

impl Bus for Machine {
    #[inline]
    fn read(&mut self, addr: u16) -> u8 {
        self.tick();
        self.read_mem(addr)
    }

    #[inline]
    fn write(&mut self, addr: u16, value: u8) {
        self.tick();
        self.write_mem(addr, value);
    }
}

pub struct Nes {
    pub cpu: Cpu,
    pub m: Machine,
}

impl Nes {
    pub fn new(rom: &[u8]) -> Result<Nes, String> {
        let cart = Cartridge::from_ines(rom)?;
        let mut nes = Nes {
            cpu: Cpu::new(),
            m: Machine {
                ppu: Ppu::new(),
                apu: Apu::new(),
                cart,
                ram: [0; 2048],
                pads: [Controller::new(), Controller::new()],
                pending_nmi: false,
                cycles: 0,
                ppu_debt: 0,
                defer: 0,
            },
        };
        nes.cpu.reset(&mut nes.m);
        nes.m.catch_up();
        Ok(nes)
    }

    pub fn reset(&mut self) {
        self.m.apu.reset();
        for _ in 0..7 {
            self.m.tick_sync();
        }
        self.m.ppu.nmi = false;
        self.m.pending_nmi = false;
        self.cpu.reset(&mut self.m);
        self.m.catch_up();
    }

    pub fn step_instruction(&mut self) {
        self.cpu.step(&mut self.m);
        if self.m.ppu_debt >= self.m.defer {
            self.m.catch_up();
        }
        self.m.latch_nmi();
        if self.m.pending_nmi {
            self.m.pending_nmi = false;
            self.cpu.nmi_pending = true;
        }
        self.cpu.irq_line = self.m.apu.irq() || self.m.cart.irq();
    }

    pub fn run_frame(&mut self) {
        while !self.m.ppu.frame_ready {
            self.step_instruction();
        }
        self.m.ppu.frame_ready = false;
    }

    pub fn frame(&self) -> &[u32] {
        &self.m.ppu.frame
    }

    pub fn set_buttons(&mut self, pad: usize, bits: u8) {
        self.m.pads[pad & 1].set_buttons(bits);
    }

    pub fn take_samples(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.m.apu.samples)
    }

    pub fn save_state(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(0x4E455331);
        self.cpu.save(&mut w);
        self.m.ppu.save(&mut w);
        self.m.apu.save(&mut w);
        self.m.cart.save(&mut w);
        w.bytes(&self.m.ram);
        self.m.pads[0].save(&mut w);
        self.m.pads[1].save(&mut w);
        w.bool(self.m.pending_nmi);
        w.u64(self.m.cycles);
        w.u32(self.m.ppu_debt);
        w.data
    }

    pub fn load_state(&mut self, data: &[u8]) -> bool {
        let mut r = Reader::new(data);
        if r.u32() != 0x4E455331 {
            return false;
        }
        self.cpu.load(&mut r);
        self.m.ppu.load(&mut r);
        self.m.apu.load(&mut r);
        self.m.cart.load(&mut r);
        r.bytes(&mut self.m.ram);
        self.m.pads[0].load(&mut r);
        self.m.pads[1].load(&mut r);
        self.m.pending_nmi = r.bool();
        self.m.cycles = r.u64();
        self.m.ppu_debt = r.u32();
        self.m.defer = 0;
        r.done()
    }
}
