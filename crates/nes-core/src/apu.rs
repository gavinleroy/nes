use crate::state::{Reader, Writer};

const CPU_HZ: u64 = 1789773;
const SAMPLE_RATE: u64 = 44100;

const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

const DUTY_TABLE: [u8; 4] = [0b0100_0000, 0b0110_0000, 0b0111_1000, 0b1001_1111];

const TRIANGLE_SEQ: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

const NOISE_PERIODS: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

const DMC_RATES: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];

fn duty_bit(duty: u8, step: u8) -> u8 {
    (DUTY_TABLE[duty as usize] >> (7 - step)) & 1
}

fn sweep_target(period: u16, shift: u8, negate: bool, ones_complement: bool) -> i32 {
    let change = (period >> shift) as i32;
    if negate {
        period as i32 - change - ones_complement as i32
    } else {
        period as i32 + change
    }
}

fn sweep_muted(period: u16, target: i32) -> bool {
    period < 8 || target > 0x7FF
}

fn advance_split(timer: u16, cycle: u32, n: u32) -> (u16, u32) {
    let t = timer as u32;
    if n <= t {
        return ((t - n) as u16, 0);
    }
    let m = n - t - 1;
    let steps = 1 + m / cycle;
    let new_timer = (cycle - 1 - m % cycle) as u16;
    (new_timer, steps)
}

fn noise_step(lfsr: u16, mode: bool) -> u16 {
    let tap = if mode { 6 } else { 1 };
    let feedback = (lfsr ^ (lfsr >> tap)) & 1;
    (lfsr >> 1) | (feedback << 14)
}

fn next_frame_event(counter: u32, mode5: bool) -> u32 {
    if counter < 7457 {
        7457
    } else if counter < 14913 {
        14913
    } else if counter < 22371 {
        22371
    } else if mode5 {
        if counter < 37281 {
            37281
        } else {
            37282
        }
    } else if counter < 29828 {
        29828
    } else if counter < 29829 {
        29829
    } else {
        29830
    }
}

fn mix(p1: u8, p2: u8, t: u8, n: u8, d: u8) -> f32 {
    let p = (p1 + p2) as f32;
    let pulse = if p == 0.0 {
        0.0
    } else {
        95.88 / (8128.0 / p + 100.0)
    };
    let tnd_in = t as f32 / 8227.0 + n as f32 / 12241.0 + d as f32 / 22638.0;
    let tnd = if tnd_in == 0.0 {
        0.0
    } else {
        159.79 / (1.0 / tnd_in + 100.0)
    };
    pulse + tnd
}

struct Envelope {
    start: bool,
    divider: u8,
    decay: u8,
    period: u8,
    loop_flag: bool,
    constant: bool,
}

impl Envelope {
    fn new() -> Self {
        Envelope {
            start: false,
            divider: 0,
            decay: 0,
            period: 0,
            loop_flag: false,
            constant: false,
        }
    }

    fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = self.period;
        } else if self.divider == 0 {
            self.divider = self.period;
            if self.decay > 0 {
                self.decay -= 1;
            } else if self.loop_flag {
                self.decay = 15;
            }
        } else {
            self.divider -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.constant {
            self.period
        } else {
            self.decay
        }
    }

    fn save(&self, w: &mut Writer) {
        w.bool(self.start);
        w.u8(self.divider);
        w.u8(self.decay);
        w.u8(self.period);
        w.bool(self.loop_flag);
        w.bool(self.constant);
    }

    fn load(&mut self, r: &mut Reader) {
        self.start = r.bool();
        self.divider = r.u8();
        self.decay = r.u8();
        self.period = r.u8();
        self.loop_flag = r.bool();
        self.constant = r.bool();
    }
}

struct Pulse {
    enabled: bool,
    duty: u8,
    step: u8,
    timer: u16,
    period: u16,
    length: u8,
    envelope: Envelope,
    sweep_enabled: bool,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_divider: u8,
    sweep_reload: bool,
    ones_complement: bool,
}

impl Pulse {
    fn new(ones_complement: bool) -> Self {
        Pulse {
            enabled: false,
            duty: 0,
            step: 0,
            timer: 0,
            period: 0,
            length: 0,
            envelope: Envelope::new(),
            sweep_enabled: false,
            sweep_period: 0,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_divider: 0,
            sweep_reload: false,
            ones_complement,
        }
    }

    fn write(&mut self, reg: u16, value: u8, block_reload: bool) {
        match reg {
            0 => {
                self.duty = value >> 6;
                self.envelope.loop_flag = value & 0x20 != 0;
                self.envelope.constant = value & 0x10 != 0;
                self.envelope.period = value & 0x0F;
            }
            1 => {
                self.sweep_enabled = value & 0x80 != 0;
                self.sweep_period = (value >> 4) & 7;
                self.sweep_negate = value & 0x08 != 0;
                self.sweep_shift = value & 7;
                self.sweep_reload = true;
            }
            2 => self.period = (self.period & 0x700) | value as u16,
            _ => {
                self.period = (self.period & 0xFF) | (((value & 7) as u16) << 8);
                if self.enabled && !(block_reload && self.length > 0) {
                    self.length = LENGTH_TABLE[(value >> 3) as usize];
                }
                self.step = 0;
                self.envelope.start = true;
            }
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.length = 0;
        }
    }

    fn advance(&mut self, n: u32) {
        if n == 0 {
            return;
        }
        let (timer, steps) = advance_split(self.timer, self.period as u32 + 1, n);
        self.timer = timer;
        self.step = ((self.step as u32 + steps) & 7) as u8;
    }

    fn clock_length(&mut self) {
        if !self.envelope.loop_flag && self.length > 0 {
            self.length -= 1;
        }
    }

    fn clock_sweep(&mut self) {
        let target = sweep_target(
            self.period,
            self.sweep_shift,
            self.sweep_negate,
            self.ones_complement,
        );
        if self.sweep_divider == 0
            && self.sweep_enabled
            && self.sweep_shift > 0
            && !sweep_muted(self.period, target)
        {
            self.period = target.max(0) as u16;
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }

    fn output(&self) -> u8 {
        let target = sweep_target(
            self.period,
            self.sweep_shift,
            self.sweep_negate,
            self.ones_complement,
        );
        if self.length == 0
            || sweep_muted(self.period, target)
            || duty_bit(self.duty, self.step) == 0
        {
            0
        } else {
            self.envelope.output()
        }
    }

    fn save(&self, w: &mut Writer, pending: u32) {
        let (timer, steps) = advance_split(self.timer, self.period as u32 + 1, pending);
        w.bool(self.enabled);
        w.u8(self.duty);
        w.u8(((self.step as u32 + steps) & 7) as u8);
        w.u16(timer);
        w.u16(self.period);
        w.u8(self.length);
        self.envelope.save(w);
        w.bool(self.sweep_enabled);
        w.u8(self.sweep_period);
        w.bool(self.sweep_negate);
        w.u8(self.sweep_shift);
        w.u8(self.sweep_divider);
        w.bool(self.sweep_reload);
    }

    fn load(&mut self, r: &mut Reader) {
        self.enabled = r.bool();
        self.duty = r.u8();
        self.step = r.u8();
        self.timer = r.u16();
        self.period = r.u16();
        self.length = r.u8();
        self.envelope.load(r);
        self.sweep_enabled = r.bool();
        self.sweep_period = r.u8();
        self.sweep_negate = r.bool();
        self.sweep_shift = r.u8();
        self.sweep_divider = r.u8();
        self.sweep_reload = r.bool();
    }
}

struct Triangle {
    enabled: bool,
    control: bool,
    linear_reload_value: u8,
    linear: u8,
    linear_reload: bool,
    period: u16,
    timer: u16,
    length: u8,
    step: u8,
}

impl Triangle {
    fn new() -> Self {
        Triangle {
            enabled: false,
            control: false,
            linear_reload_value: 0,
            linear: 0,
            linear_reload: false,
            period: 0,
            timer: 0,
            length: 0,
            step: 0,
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.length = 0;
        }
    }

    #[cfg(test)]
    fn clock_timer(&mut self) {
        self.advance(1);
    }

    fn advance(&mut self, n: u32) {
        if n == 0 {
            return;
        }
        let (timer, steps) = advance_split(self.timer, self.period as u32 + 1, n);
        self.timer = timer;
        if self.linear > 0 && self.length > 0 {
            self.step = ((self.step as u32 + steps) & 31) as u8;
        }
    }

    fn clock_linear(&mut self) {
        if self.linear_reload {
            self.linear = self.linear_reload_value;
        } else if self.linear > 0 {
            self.linear -= 1;
        }
        if !self.control {
            self.linear_reload = false;
        }
    }

    fn clock_length(&mut self) {
        if !self.control && self.length > 0 {
            self.length -= 1;
        }
    }

    fn output(&self) -> u8 {
        TRIANGLE_SEQ[self.step as usize]
    }

    fn save(&self, w: &mut Writer, pending: u32) {
        let (timer, steps) = advance_split(self.timer, self.period as u32 + 1, pending);
        let step = if self.linear > 0 && self.length > 0 {
            ((self.step as u32 + steps) & 31) as u8
        } else {
            self.step
        };
        w.bool(self.enabled);
        w.bool(self.control);
        w.u8(self.linear_reload_value);
        w.u8(self.linear);
        w.bool(self.linear_reload);
        w.u16(self.period);
        w.u16(timer);
        w.u8(self.length);
        w.u8(step);
    }

    fn load(&mut self, r: &mut Reader) {
        self.enabled = r.bool();
        self.control = r.bool();
        self.linear_reload_value = r.u8();
        self.linear = r.u8();
        self.linear_reload = r.bool();
        self.period = r.u16();
        self.timer = r.u16();
        self.length = r.u8();
        self.step = r.u8();
    }
}

struct Noise {
    enabled: bool,
    envelope: Envelope,
    mode: bool,
    period: u16,
    timer: u16,
    lfsr: u16,
    length: u8,
}

impl Noise {
    fn new() -> Self {
        Noise {
            enabled: false,
            envelope: Envelope::new(),
            mode: false,
            period: NOISE_PERIODS[0],
            timer: 0,
            lfsr: 1,
            length: 0,
        }
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.length = 0;
        }
    }

    fn advance(&mut self, n: u32) {
        if n == 0 {
            return;
        }
        let (timer, steps) = advance_split(self.timer, self.period as u32, n);
        self.timer = timer;
        let mut i = 0;
        while i < steps {
            self.lfsr = noise_step(self.lfsr, self.mode);
            i += 1;
        }
    }

    fn clock_length(&mut self) {
        if !self.envelope.loop_flag && self.length > 0 {
            self.length -= 1;
        }
    }

    fn output(&self) -> u8 {
        if self.length == 0 || self.lfsr & 1 == 1 {
            0
        } else {
            self.envelope.output()
        }
    }

    fn save(&self, w: &mut Writer, pending: u32) {
        let (timer, steps) = advance_split(self.timer, self.period as u32, pending);
        let mut lfsr = self.lfsr;
        let mut i = 0;
        while i < steps {
            lfsr = noise_step(lfsr, self.mode);
            i += 1;
        }
        w.bool(self.enabled);
        self.envelope.save(w);
        w.bool(self.mode);
        w.u16(self.period);
        w.u16(timer);
        w.u16(lfsr);
        w.u8(self.length);
    }

    fn load(&mut self, r: &mut Reader) {
        self.enabled = r.bool();
        self.envelope.load(r);
        self.mode = r.bool();
        self.period = r.u16();
        self.timer = r.u16();
        self.lfsr = r.u16();
        self.length = r.u8();
    }
}

struct Dmc {
    irq_enabled: bool,
    loop_flag: bool,
    period: u16,
    timer: u16,
    output: u8,
    sample_address: u16,
    sample_length: u16,
    current_address: u16,
    bytes_remaining: u16,
    shift: u8,
    bits: u8,
    silence: bool,
    buffer: Option<u8>,
    pending: bool,
    irq: bool,
}

impl Dmc {
    fn new() -> Self {
        Dmc {
            irq_enabled: false,
            loop_flag: false,
            period: DMC_RATES[0],
            timer: 0,
            output: 0,
            sample_address: 0xC000,
            sample_length: 1,
            current_address: 0xC000,
            bytes_remaining: 0,
            shift: 0,
            bits: 8,
            silence: true,
            buffer: None,
            pending: false,
            irq: false,
        }
    }

    fn write(&mut self, reg: u16, value: u8) {
        match reg {
            0 => {
                self.irq_enabled = value & 0x80 != 0;
                if !self.irq_enabled {
                    self.irq = false;
                }
                self.loop_flag = value & 0x40 != 0;
                self.period = DMC_RATES[(value & 0x0F) as usize];
            }
            1 => self.output = value & 0x7F,
            2 => self.sample_address = 0xC000 + 64 * value as u16,
            _ => self.sample_length = 16 * value as u16 + 1,
        }
    }

    fn restart(&mut self) {
        self.current_address = self.sample_address;
        self.bytes_remaining = self.sample_length;
    }

    #[inline(always)]
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.period - 1;
            self.clock_output();
        } else {
            self.timer -= 1;
        }
    }

    fn clock_output(&mut self) {
        if !self.silence {
            if self.shift & 1 == 1 {
                if self.output <= 125 {
                    self.output += 2;
                }
            } else if self.output >= 2 {
                self.output -= 2;
            }
        }
        self.shift >>= 1;
        self.bits -= 1;
        if self.bits == 0 {
            self.bits = 8;
            match self.buffer.take() {
                Some(b) => {
                    self.shift = b;
                    self.silence = false;
                }
                None => self.silence = true,
            }
        }
    }

    fn supply(&mut self, value: u8) {
        self.buffer = Some(value);
        self.pending = false;
        if self.bytes_remaining > 0 {
            self.current_address = if self.current_address == 0xFFFF {
                0x8000
            } else {
                self.current_address + 1
            };
            self.bytes_remaining -= 1;
            if self.bytes_remaining == 0 {
                if self.loop_flag {
                    self.restart();
                } else if self.irq_enabled {
                    self.irq = true;
                }
            }
        }
    }

    fn save(&self, w: &mut Writer) {
        w.bool(self.irq_enabled);
        w.bool(self.loop_flag);
        w.u16(self.period);
        w.u16(self.timer);
        w.u8(self.output);
        w.u16(self.sample_address);
        w.u16(self.sample_length);
        w.u16(self.current_address);
        w.u16(self.bytes_remaining);
        w.u8(self.shift);
        w.u8(self.bits);
        w.bool(self.silence);
        w.bool(self.buffer.is_some());
        w.u8(self.buffer.unwrap_or(0));
        w.bool(self.pending);
        w.bool(self.irq);
    }

    fn load(&mut self, r: &mut Reader) {
        self.irq_enabled = r.bool();
        self.loop_flag = r.bool();
        self.period = r.u16();
        self.timer = r.u16();
        self.output = r.u8();
        self.sample_address = r.u16();
        self.sample_length = r.u16();
        self.current_address = r.u16();
        self.bytes_remaining = r.u16();
        self.shift = r.u8();
        self.bits = r.u8();
        self.silence = r.bool();
        let has_buffer = r.bool();
        let buffer_value = r.u8();
        self.buffer = if has_buffer { Some(buffer_value) } else { None };
        self.pending = r.bool();
        self.irq = r.bool();
    }
}

pub struct Apu {
    pub samples: Vec<f32>,
    pulse1: Pulse,
    pulse2: Pulse,
    triangle: Triangle,
    noise: Noise,
    dmc: Dmc,
    frame_counter: u32,
    next_event: u32,
    mode5: bool,
    irq_inhibit: bool,
    frame_irq: bool,
    odd_cycle: bool,
    sample_acc: u64,
    write_delay: u8,
    write_value: u8,
    half_clocked: bool,
    lag: u32,
}

impl Default for Apu {
    fn default() -> Self {
        Apu::new()
    }
}

impl Apu {
    pub fn new() -> Self {
        Apu {
            samples: Vec::new(),
            pulse1: Pulse::new(true),
            pulse2: Pulse::new(false),
            triangle: Triangle::new(),
            noise: Noise::new(),
            dmc: Dmc::new(),
            frame_counter: 4,
            next_event: 7457,
            mode5: false,
            irq_inhibit: false,
            frame_irq: false,
            odd_cycle: false,
            sample_acc: 0,
            write_delay: 0,
            write_value: 0,
            half_clocked: false,
            lag: 0,
        }
    }

    pub fn reset(&mut self) {
        self.write(0x4015, 0);
        self.frame_irq = false;
        self.dmc.irq = false;
        self.pulse1.envelope.loop_flag = false;
        self.pulse2.envelope.loop_flag = false;
        self.noise.envelope.loop_flag = false;
        let value = self.write_value;
        self.write(0x4017, value);
    }

    #[inline(always)]
    pub fn tick(&mut self) {
        self.half_clocked = false;
        self.frame_counter += 1;
        if self.frame_counter == self.next_event {
            self.flush();
            self.frame_event();
        }
        if self.write_delay > 0 {
            self.write_delay -= 1;
            if self.write_delay == 0 {
                self.flush();
                self.mode5 = self.write_value & 0x80 != 0;
                self.frame_counter = 0;
                self.next_event = 7457;
                if self.mode5 {
                    self.quarter_frame();
                    self.half_frame();
                }
            }
        }
        self.odd_cycle = !self.odd_cycle;
        self.lag += 1;
        self.dmc.clock_timer();
        self.sample_acc += SAMPLE_RATE;
        if self.sample_acc >= CPU_HZ {
            self.sample_acc -= CPU_HZ;
            self.flush();
            self.samples.push(mix(
                self.pulse1.output(),
                self.pulse2.output(),
                self.triangle.output(),
                self.noise.output(),
                self.dmc.output,
            ));
        }
    }

    fn flush(&mut self) {
        let lag = self.lag;
        if lag == 0 {
            return;
        }
        self.lag = 0;
        let pulse_clocks = lag / 2 + ((lag & 1) & self.odd_cycle as u32);
        self.pulse1.advance(pulse_clocks);
        self.pulse2.advance(pulse_clocks);
        self.triangle.advance(lag);
        self.noise.advance(lag);
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        self.flush();
        match addr {
            0x4000..=0x4003 => self.pulse1.write(addr & 3, value, self.half_clocked),
            0x4004..=0x4007 => self.pulse2.write(addr & 3, value, self.half_clocked),
            0x4008 => {
                self.triangle.control = value & 0x80 != 0;
                self.triangle.linear_reload_value = value & 0x7F;
            }
            0x400A => self.triangle.period = (self.triangle.period & 0x700) | value as u16,
            0x400B => {
                self.triangle.period = (self.triangle.period & 0xFF) | (((value & 7) as u16) << 8);
                if self.triangle.enabled && !(self.half_clocked && self.triangle.length > 0) {
                    self.triangle.length = LENGTH_TABLE[(value >> 3) as usize];
                }
                self.triangle.linear_reload = true;
            }
            0x400C => {
                self.noise.envelope.loop_flag = value & 0x20 != 0;
                self.noise.envelope.constant = value & 0x10 != 0;
                self.noise.envelope.period = value & 0x0F;
            }
            0x400E => {
                self.noise.mode = value & 0x80 != 0;
                self.noise.period = NOISE_PERIODS[(value & 0x0F) as usize];
            }
            0x400F => {
                if self.noise.enabled && !(self.half_clocked && self.noise.length > 0) {
                    self.noise.length = LENGTH_TABLE[(value >> 3) as usize];
                }
                self.noise.envelope.start = true;
            }
            0x4010..=0x4013 => self.dmc.write(addr & 3, value),
            0x4015 => {
                self.pulse1.set_enabled(value & 0x01 != 0);
                self.pulse2.set_enabled(value & 0x02 != 0);
                self.triangle.set_enabled(value & 0x04 != 0);
                self.noise.set_enabled(value & 0x08 != 0);
                self.dmc.irq = false;
                if value & 0x10 == 0 {
                    self.dmc.bytes_remaining = 0;
                } else if self.dmc.bytes_remaining == 0 {
                    self.dmc.restart();
                }
            }
            0x4017 => {
                self.irq_inhibit = value & 0x40 != 0;
                if self.irq_inhibit {
                    self.frame_irq = false;
                }
                self.write_value = value;
                self.write_delay = if self.odd_cycle { 3 } else { 4 };
            }
            _ => {}
        }
    }

    pub fn read_status(&mut self) -> u8 {
        let mut v = 0;
        if self.pulse1.length > 0 {
            v |= 0x01;
        }
        if self.pulse2.length > 0 {
            v |= 0x02;
        }
        if self.triangle.length > 0 {
            v |= 0x04;
        }
        if self.noise.length > 0 {
            v |= 0x08;
        }
        if self.dmc.bytes_remaining > 0 {
            v |= 0x10;
        }
        if self.frame_irq {
            v |= 0x40;
        }
        if self.dmc.irq {
            v |= 0x80;
        }
        self.frame_irq = false;
        v
    }

    pub fn irq(&self) -> bool {
        self.frame_irq || self.dmc.irq
    }

    #[inline(always)]
    pub fn dmc_hungry(&self) -> bool {
        self.dmc.bytes_remaining > 0 && !self.dmc.pending && self.dmc.buffer.is_none()
    }

    pub fn dmc_request(&mut self) -> Option<u16> {
        if self.dmc.buffer.is_none() && self.dmc.bytes_remaining > 0 && !self.dmc.pending {
            self.dmc.pending = true;
            Some(self.dmc.current_address)
        } else {
            None
        }
    }

    pub fn dmc_supply(&mut self, value: u8) {
        self.dmc.supply(value);
    }

    pub fn save(&self, w: &mut Writer) {
        w.u32(self.frame_counter);
        w.bool(self.mode5);
        w.bool(self.irq_inhibit);
        w.bool(self.frame_irq);
        w.bool(self.odd_cycle);
        w.u64(self.sample_acc);
        w.u8(self.write_delay);
        w.u8(self.write_value);
        w.bool(self.half_clocked);
        let lag = self.lag;
        let pulse_clocks = lag / 2 + ((lag & 1) & self.odd_cycle as u32);
        self.pulse1.save(w, pulse_clocks);
        self.pulse2.save(w, pulse_clocks);
        self.triangle.save(w, lag);
        self.noise.save(w, lag);
        self.dmc.save(w);
    }

    pub fn load(&mut self, r: &mut Reader) {
        self.frame_counter = r.u32();
        self.mode5 = r.bool();
        self.next_event = next_frame_event(self.frame_counter, self.mode5);
        self.irq_inhibit = r.bool();
        self.frame_irq = r.bool();
        self.odd_cycle = r.bool();
        self.sample_acc = r.u64();
        self.write_delay = r.u8();
        self.write_value = r.u8();
        self.half_clocked = r.bool();
        self.lag = 0;
        self.pulse1.load(r);
        self.pulse2.load(r);
        self.triangle.load(r);
        self.noise.load(r);
        self.dmc.load(r);
        self.samples.clear();
    }

    fn quarter_frame(&mut self) {
        self.pulse1.envelope.clock();
        self.pulse2.envelope.clock();
        self.noise.envelope.clock();
        self.triangle.clock_linear();
    }

    fn half_frame(&mut self) {
        self.half_clocked = true;
        self.pulse1.clock_length();
        self.pulse1.clock_sweep();
        self.pulse2.clock_length();
        self.pulse2.clock_sweep();
        self.triangle.clock_length();
        self.noise.clock_length();
    }

    fn set_frame_irq(&mut self) {
        if !self.irq_inhibit {
            self.frame_irq = true;
        }
    }

    fn frame_event(&mut self) {
        if self.mode5 {
            match self.frame_counter {
                7457 | 22371 => self.quarter_frame(),
                14913 | 37281 => {
                    self.quarter_frame();
                    self.half_frame();
                }
                _ => self.frame_counter = 0,
            }
        } else {
            match self.frame_counter {
                7457 | 22371 => self.quarter_frame(),
                14913 => {
                    self.quarter_frame();
                    self.half_frame();
                }
                29828 => self.set_frame_irq(),
                29829 => {
                    self.quarter_frame();
                    self.half_frame();
                    self.set_frame_irq();
                }
                _ => {
                    self.set_frame_irq();
                    self.frame_counter = 0;
                }
            }
        }
        self.next_event = next_frame_event(self.frame_counter, self.mode5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn length_table_matches_canonical() {
        let canonical: [u8; 32] = [
            10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20,
            96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
        ];
        assert_eq!(LENGTH_TABLE, canonical);
    }

    #[test]
    fn duty_sequences_match_canonical() {
        let canonical: [[u8; 8]; 4] = [
            [0, 1, 0, 0, 0, 0, 0, 0],
            [0, 1, 1, 0, 0, 0, 0, 0],
            [0, 1, 1, 1, 1, 0, 0, 0],
            [1, 0, 0, 1, 1, 1, 1, 1],
        ];
        for duty in 0..4u8 {
            for step in 0..8u8 {
                assert_eq!(
                    duty_bit(duty, step),
                    canonical[duty as usize][step as usize],
                    "duty {} step {}",
                    duty,
                    step
                );
            }
        }
    }

    #[test]
    fn triangle_sequence_is_cyclic_ramp() {
        for (i, &v) in TRIANGLE_SEQ.iter().enumerate() {
            let expected = if i < 16 { 15 - i } else { i - 16 } as u8;
            assert_eq!(v, expected);
        }
        let mut tri = Triangle::new();
        tri.enabled = true;
        tri.length = 100;
        tri.linear = 100;
        tri.period = 0;
        let mut seen = Vec::new();
        for _ in 0..96 {
            seen.push(tri.output());
            tri.clock_timer();
        }
        for (i, &v) in seen.iter().enumerate() {
            assert_eq!(v, TRIANGLE_SEQ[i % 32]);
        }
    }

    #[test]
    fn triangle_holds_output_when_counters_zero() {
        let mut tri = Triangle::new();
        tri.period = 0;
        tri.step = 5;
        tri.length = 0;
        tri.linear = 10;
        for _ in 0..10 {
            tri.clock_timer();
            assert_eq!(tri.step, 5);
        }
        tri.length = 10;
        tri.linear = 0;
        for _ in 0..10 {
            tri.clock_timer();
            assert_eq!(tri.step, 5);
        }
    }

    #[test]
    fn sweep_target_exhaustive_both_channels() {
        for period in 0..=0x7FFu16 {
            for shift in 0..=7u8 {
                let change = (period >> shift) as i32;
                let add = period as i32 + change;
                let twos = period as i32 - change;
                let ones = period as i32 - change - 1;
                for &ones_complement in &[true, false] {
                    assert_eq!(sweep_target(period, shift, false, ones_complement), add);
                }
                assert_eq!(sweep_target(period, shift, true, true), ones);
                assert_eq!(sweep_target(period, shift, true, false), twos);
                assert_eq!(
                    sweep_target(period, shift, true, false)
                        - sweep_target(period, shift, true, true),
                    1
                );
                for &(negate, oc) in &[(false, false), (false, true), (true, false), (true, true)] {
                    let t = sweep_target(period, shift, negate, oc);
                    assert_eq!(sweep_muted(period, t), period < 8 || t > 0x7FF);
                }
            }
        }
    }

    #[test]
    fn envelope_decay_property_all_periods() {
        for period in 0..=15u8 {
            for &loop_flag in &[false, true] {
                let mut env = Envelope::new();
                env.period = period;
                env.loop_flag = loop_flag;
                env.start = true;
                let total = (period as u32 + 1) * 40 + 2;
                for c in 1..=total {
                    env.clock();
                    let steps = (c - 1) / (period as u32 + 1);
                    let expected = if loop_flag {
                        15 - (steps % 16) as u8
                    } else {
                        15u8.saturating_sub(steps.min(15) as u8)
                    };
                    assert_eq!(
                        env.decay, expected,
                        "period {} loop {} clock {}",
                        period, loop_flag, c
                    );
                }
            }
        }
    }

    #[test]
    fn envelope_constant_volume_output() {
        let mut env = Envelope::new();
        env.period = 9;
        env.constant = true;
        env.decay = 3;
        assert_eq!(env.output(), 9);
        env.constant = false;
        assert_eq!(env.output(), 3);
    }

    #[test]
    fn noise_lfsr_feedback_and_period() {
        for &mode in &[false, true] {
            let tap = if mode { 6 } else { 1 };
            let mut lfsr = 1u16;
            for _ in 0..40000 {
                let next = noise_step(lfsr, mode);
                let feedback = (lfsr & 1) ^ ((lfsr >> tap) & 1);
                assert_eq!(next & 0x3FFF, lfsr >> 1);
                assert_eq!((next >> 14) & 1, feedback);
                assert!(next != 0);
                assert!(next < 0x8000);
                lfsr = next;
            }
        }
        let mut lfsr = 1u16;
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(lfsr) {
                break;
            }
            lfsr = noise_step(lfsr, false);
        }
        assert_eq!(seen.len(), 32767);
        assert_eq!(lfsr, 1);
    }

    #[test]
    fn mixer_zero_and_monotonic() {
        assert_eq!(mix(0, 0, 0, 0, 0), 0.0);
        let vol_grid = [0u8, 1, 7, 14, 15];
        let dmc_grid = [0u8, 1, 64, 126, 127];
        for &a in &vol_grid {
            for &t in &vol_grid {
                for &n in &vol_grid {
                    for &d in &dmc_grid {
                        for p in 0..15u8 {
                            assert!(mix(p + 1, a, t, n, d) > mix(p, a, t, n, d));
                            assert!(mix(a, p + 1, t, n, d) > mix(a, p, t, n, d));
                            assert!(mix(a, n, p + 1, t, d) > mix(a, n, p, t, d));
                            assert!(mix(a, n, t, p + 1, d) > mix(a, n, t, p, d));
                        }
                        for dd in 0..127u8 {
                            assert!(mix(a, t, n, d, dd + 1) > mix(a, t, n, d, dd));
                        }
                        let v = mix(a, a, t, n, d);
                        assert!((0.0..1.0).contains(&v));
                    }
                }
            }
        }
    }

    fn frame_event_spec(mode5_write: bool, n: u64) -> (u64, u64) {
        let mut q = 0u64;
        let mut h = 0u64;
        let mut c = 4u32;
        let mut mode5 = false;
        let mut delay = if mode5_write { 4u32 } else { 0 };
        for _ in 0..n {
            c += 1;
            if mode5 {
                match c {
                    7457 | 22371 => q += 1,
                    14913 | 37281 => {
                        q += 1;
                        h += 1;
                    }
                    37282 => c = 0,
                    _ => {}
                }
            } else {
                match c {
                    7457 | 22371 => q += 1,
                    14913 | 29829 => {
                        q += 1;
                        h += 1;
                    }
                    29830 => c = 0,
                    _ => {}
                }
            }
            if delay > 0 {
                delay -= 1;
                if delay == 0 {
                    mode5 = mode5_write;
                    c = 0;
                    if mode5 {
                        q += 1;
                        h += 1;
                    }
                }
            }
        }
        (q, h)
    }

    fn count_frame_events(apu: &mut Apu, n: u64) -> (u64, u64) {
        apu.pulse1.envelope.loop_flag = true;
        apu.pulse1.envelope.decay = 15;
        apu.pulse1.envelope.divider = 0;
        apu.pulse1.envelope.period = 0;
        apu.pulse1.envelope.start = false;
        apu.pulse2.enabled = true;
        apu.pulse2.length = 250;
        apu.pulse2.envelope.loop_flag = false;
        let mut q = 0;
        let mut h = 0;
        let mut prev_decay = apu.pulse1.envelope.decay;
        let mut prev_len = apu.pulse2.length;
        for _ in 0..n {
            apu.tick();
            if apu.pulse1.envelope.decay != prev_decay {
                q += 1;
                prev_decay = apu.pulse1.envelope.decay;
            }
            if apu.pulse2.length != prev_len {
                h += 1;
                prev_len = apu.pulse2.length;
            }
        }
        (q, h)
    }

    #[test]
    fn frame_counter_four_step_schedule() {
        for &n in &[7456u64, 7457, 14913, 22371, 29829, 29830, 100000] {
            let mut apu = Apu::new();
            let observed = count_frame_events(&mut apu, n);
            assert_eq!(observed, frame_event_spec(false, n), "n = {}", n);
        }
    }

    #[test]
    fn frame_counter_five_step_schedule() {
        for &n in &[7456u64, 14913, 29829, 37281, 37282, 120000] {
            let mut apu = Apu::new();
            apu.write(0x4017, 0x80);
            let observed = count_frame_events(&mut apu, n);
            assert_eq!(observed, frame_event_spec(true, n), "n = {}", n);
        }
    }

    #[test]
    fn write_4017_bit7_clocks_after_delay() {
        let mut apu = Apu::new();
        apu.write(0x4015, 0x01);
        apu.write(0x4000, 0x00);
        apu.write(0x4003, 0x08);
        let len = apu.pulse1.length;
        apu.write(0x4017, 0x80);
        for _ in 0..3 {
            apu.tick();
            assert_eq!(apu.pulse1.length, len);
        }
        apu.tick();
        assert_eq!(apu.pulse1.length, len - 1);
        assert_eq!(apu.pulse1.envelope.decay, 15);
        apu.write(0x4017, 0x00);
        for _ in 0..8 {
            apu.tick();
        }
        assert_eq!(apu.pulse1.length, len - 1);
        let mut apu = Apu::new();
        apu.tick();
        apu.write(0x4015, 0x01);
        apu.write(0x4000, 0x00);
        apu.write(0x4003, 0x08);
        let len = apu.pulse1.length;
        apu.write(0x4017, 0x80);
        for _ in 0..2 {
            apu.tick();
            assert_eq!(apu.pulse1.length, len);
        }
        apu.tick();
        assert_eq!(apu.pulse1.length, len - 1);
    }

    #[test]
    fn frame_irq_set_and_cleared() {
        let mut apu = Apu::new();
        for _ in 0..29823 {
            apu.tick();
        }
        assert!(!apu.irq());
        apu.tick();
        assert!(apu.irq());
        assert!(apu.read_status() & 0x40 != 0);
        assert!(!apu.irq());
        apu.tick();
        assert!(apu.irq());
        apu.tick();
        apu.tick();
        assert!(apu.irq());
        let status = apu.read_status();
        assert!(status & 0x40 != 0);
        assert!(!apu.irq());
        assert!(apu.read_status() & 0x40 == 0);
        let mut apu = Apu::new();
        apu.write(0x4017, 0x40);
        for _ in 0..40000 {
            apu.tick();
        }
        assert!(!apu.irq());
        let mut apu = Apu::new();
        apu.write(0x4017, 0x80);
        for _ in 0..40000 {
            apu.tick();
        }
        assert!(!apu.irq());
    }

    #[test]
    fn status_reflects_lengths_and_enable_clears() {
        let mut apu = Apu::new();
        apu.write(0x4015, 0x0F);
        apu.write(0x4003, 0x08);
        apu.write(0x4007, 0x08);
        apu.write(0x400B, 0x08);
        apu.write(0x400F, 0x08);
        assert_eq!(apu.read_status() & 0x0F, 0x0F);
        apu.write(0x4015, 0x05);
        assert_eq!(apu.read_status() & 0x0F, 0x05);
        apu.write(0x4015, 0x00);
        assert_eq!(apu.read_status() & 0x1F, 0x00);
        let mut apu = Apu::new();
        apu.write(0x4003, 0x08);
        assert_eq!(apu.read_status() & 0x01, 0x00);
    }

    #[test]
    fn dmc_requests_one_sample_length_for_all_rates() {
        for rate in 0..16u8 {
            let mut apu = Apu::new();
            apu.write(0x4010, rate);
            apu.write(0x4012, 0x10);
            apu.write(0x4013, 0x01);
            apu.write(0x4015, 0x10);
            let expected_len = 17u64;
            let ticks = 8 * DMC_RATES[rate as usize] as u64 * (expected_len + 2);
            let mut requests = 0u64;
            for _ in 0..ticks {
                if apu.dmc_request().is_some() {
                    requests += 1;
                    apu.dmc_supply(0x55);
                }
                apu.tick();
            }
            assert_eq!(requests, expected_len, "rate {}", rate);
            assert_eq!(apu.read_status() & 0x10, 0);
        }
    }

    #[test]
    fn dmc_address_wraps_ffff_to_8000() {
        let mut apu = Apu::new();
        apu.write(0x4010, 0x0F);
        apu.write(0x4012, 0xFF);
        apu.write(0x4013, 0xFF);
        apu.write(0x4015, 0x10);
        let expected_len = 16 * 255 + 1;
        let mut addrs = Vec::new();
        let ticks = 8 * 54 * (expected_len as u64 + 2);
        for _ in 0..ticks {
            if let Some(a) = apu.dmc_request() {
                addrs.push(a);
                apu.dmc_supply(0);
            }
            apu.tick();
        }
        assert_eq!(addrs.len(), expected_len);
        assert_eq!(addrs[0], 0xFFC0);
        assert_eq!(addrs[63], 0xFFFF);
        assert_eq!(addrs[64], 0x8000);
        for i in 1..addrs.len() {
            let expected = if addrs[i - 1] == 0xFFFF {
                0x8000
            } else {
                addrs[i - 1] + 1
            };
            assert_eq!(addrs[i], expected);
        }
    }

    #[test]
    fn dmc_delta_output_bit_steps_exhaustive() {
        for byte in 0..=255u8 {
            for &start in &[0u8, 1, 2, 63, 124, 125, 126, 127] {
                let mut dmc = Dmc::new();
                dmc.output = start;
                dmc.shift = byte;
                dmc.silence = false;
                dmc.bits = 8;
                let mut expected = start;
                for bit in 0..8 {
                    dmc.clock_output();
                    if (byte >> bit) & 1 == 1 {
                        if expected <= 125 {
                            expected += 2;
                        }
                    } else if expected >= 2 {
                        expected -= 2;
                    }
                    assert_eq!(dmc.output, expected);
                    assert!(dmc.output <= 127);
                }
            }
        }
    }

    #[test]
    fn dmc_irq_and_loop_behavior() {
        let mut apu = Apu::new();
        apu.write(0x4010, 0x8F);
        apu.write(0x4012, 0x00);
        apu.write(0x4013, 0x00);
        apu.write(0x4015, 0x10);
        let addr = apu.dmc_request().unwrap();
        assert_eq!(addr, 0xC000);
        apu.dmc_supply(0xFF);
        assert!(apu.irq());
        assert!(apu.read_status() & 0x80 != 0);
        assert!(apu.read_status() & 0x80 != 0);
        apu.write(0x4015, 0x10);
        assert!(!apu.irq());
        assert!(apu.dmc_request().is_none());
        let mut apu = Apu::new();
        apu.write(0x4010, 0x4F);
        apu.write(0x4013, 0x00);
        apu.write(0x4015, 0x10);
        apu.dmc_request().unwrap();
        apu.dmc_supply(0);
        assert!(!apu.irq());
        assert_eq!(apu.read_status() & 0x10, 0x10);
    }

    #[test]
    fn sample_pacing_matches_rate() {
        let mut apu = Apu::new();
        for _ in 0..CPU_HZ {
            apu.tick();
        }
        assert_eq!(apu.samples.len(), SAMPLE_RATE as usize);
        for &s in &apu.samples {
            assert!((0.0..1.0).contains(&s));
        }
    }

    #[test]
    fn save_load_roundtrip_preserves_behavior() {
        let mut apu = Apu::new();
        apu.write(0x4015, 0x1F);
        apu.write(0x4000, 0xBF);
        apu.write(0x4001, 0x91);
        apu.write(0x4002, 0x42);
        apu.write(0x4003, 0x31);
        apu.write(0x4004, 0x7A);
        apu.write(0x4006, 0x80);
        apu.write(0x4007, 0x12);
        apu.write(0x4008, 0x81);
        apu.write(0x400A, 0x55);
        apu.write(0x400B, 0x23);
        apu.write(0x400C, 0x05);
        apu.write(0x400E, 0x83);
        apu.write(0x400F, 0x40);
        apu.write(0x4010, 0x45);
        apu.write(0x4012, 0x20);
        apu.write(0x4013, 0x02);
        apu.write(0x4015, 0x1F);
        apu.write(0x4017, 0x80);
        for i in 0..50000u64 {
            if apu.dmc_request().is_some() {
                apu.dmc_supply(i as u8);
            }
            apu.tick();
        }
        let mut w = Writer::new();
        apu.save(&mut w);
        let mut apu2 = Apu::new();
        apu2.samples.push(9.0);
        let mut r = Reader::new(&w.data);
        apu2.load(&mut r);
        assert!(r.done());
        assert!(apu2.samples.is_empty());
        let mut w2 = Writer::new();
        apu2.save(&mut w2);
        assert_eq!(w.data, w2.data);
        apu.samples.clear();
        for i in 0..50000u64 {
            let r1 = apu.dmc_request();
            let r2 = apu2.dmc_request();
            assert_eq!(r1, r2);
            if r1.is_some() {
                apu.dmc_supply(i as u8);
                apu2.dmc_supply(i as u8);
            }
            apu.tick();
            apu2.tick();
        }
        assert_eq!(apu.samples, apu2.samples);
        assert_eq!(apu.irq(), apu2.irq());
    }

    #[test]
    fn pulse_mutes_below_period_8_and_sweep_overflow() {
        let mut p = Pulse::new(true);
        p.enabled = true;
        p.length = 10;
        p.envelope.constant = true;
        p.envelope.period = 12;
        p.duty = 3;
        p.step = 0;
        p.period = 7;
        assert_eq!(p.output(), 0);
        p.period = 8;
        assert_eq!(p.output(), 12);
        p.period = 0x700;
        p.sweep_shift = 1;
        p.sweep_negate = false;
        assert_eq!(p.output(), 0);
        p.sweep_negate = true;
        assert_eq!(p.output(), 12);
        p.length = 0;
        assert_eq!(p.output(), 0);
    }

    #[test]
    fn sweep_updates_period_with_channel_difference() {
        for &(ones, expected) in &[(true, 0x200 - 0x40 - 1), (false, 0x200 - 0x40)] {
            let mut p = Pulse::new(ones);
            p.enabled = true;
            p.period = 0x200;
            p.sweep_enabled = true;
            p.sweep_negate = true;
            p.sweep_shift = 3;
            p.sweep_period = 0;
            p.sweep_divider = 0;
            p.clock_sweep();
            assert_eq!(p.period as i32, expected);
        }
    }
}
