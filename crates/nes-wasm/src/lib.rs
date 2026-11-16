#![allow(clippy::missing_safety_doc)]

use nes_core::Nes;
use std::sync::Mutex;

static NES: Mutex<Option<Nes>> = Mutex::new(None);
static STATE: Mutex<Vec<u8>> = Mutex::new(Vec::new());
static SAMPLES: Mutex<Vec<f32>> = Mutex::new(Vec::new());
static FRAME: Mutex<Vec<u32>> = Mutex::new(Vec::new());

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn load_rom(ptr: *const u8, len: usize) -> i32 {
    let rom = std::slice::from_raw_parts(ptr, len);
    match Nes::new(rom) {
        Ok(nes) => {
            *NES.lock().unwrap() = Some(nes);
            0
        }
        Err(_) => 1,
    }
}

#[no_mangle]
pub extern "C" fn run_frame() {
    if let Some(nes) = NES.lock().unwrap().as_mut() {
        nes.run_frame();
        let mut frame = FRAME.lock().unwrap();
        frame.clear();
        frame.extend_from_slice(nes.frame());
        let mut samples = SAMPLES.lock().unwrap();
        samples.clear();
        samples.extend(nes.take_samples());
    }
}

#[no_mangle]
pub extern "C" fn reset() {
    if let Some(nes) = NES.lock().unwrap().as_mut() {
        nes.reset();
    }
}

#[no_mangle]
pub extern "C" fn frame_ptr() -> *const u32 {
    FRAME.lock().unwrap().as_ptr()
}

#[no_mangle]
pub extern "C" fn frame_len() -> usize {
    FRAME.lock().unwrap().len()
}

#[no_mangle]
pub extern "C" fn samples_ptr() -> *const f32 {
    SAMPLES.lock().unwrap().as_ptr()
}

#[no_mangle]
pub extern "C" fn samples_len() -> usize {
    SAMPLES.lock().unwrap().len()
}

#[no_mangle]
pub extern "C" fn set_buttons(pad: u32, bits: u32) {
    if let Some(nes) = NES.lock().unwrap().as_mut() {
        nes.set_buttons(pad as usize, bits as u8);
    }
}

#[no_mangle]
pub extern "C" fn save_state() -> usize {
    let mut state = STATE.lock().unwrap();
    state.clear();
    if let Some(nes) = NES.lock().unwrap().as_ref() {
        *state = nes.save_state();
    }
    state.len()
}

#[no_mangle]
pub extern "C" fn state_ptr() -> *const u8 {
    STATE.lock().unwrap().as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn load_state(ptr: *const u8, len: usize) -> i32 {
    let data = std::slice::from_raw_parts(ptr, len);
    match NES.lock().unwrap().as_mut() {
        Some(nes) => !nes.load_state(data) as i32,
        None => 1,
    }
}
