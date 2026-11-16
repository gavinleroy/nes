use nes_core::nes::{Nes, HEIGHT, WIDTH};
use sdl2::audio::{AudioQueue, AudioSpecDesired};
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;
use std::time::Duration;

const BUTTONS: [(Keycode, u8); 8] = [
    (Keycode::Z, 0x01),
    (Keycode::X, 0x02),
    (Keycode::RShift, 0x04),
    (Keycode::Return, 0x08),
    (Keycode::Up, 0x10),
    (Keycode::Down, 0x20),
    (Keycode::Left, 0x40),
    (Keycode::Right, 0x80),
];

fn main() -> Result<(), String> {
    let rom_path = std::env::args()
        .nth(1)
        .ok_or("usage: nes-native <rom.nes>")?;
    let rom = std::fs::read(&rom_path).map_err(|e| format!("{rom_path}: {e}"))?;
    let mut nes = Nes::new(&rom)?;
    let state_path = format!("{rom_path}.state");

    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let window = video
        .window("nes", (WIDTH * 3) as u32, (HEIGHT * 3) as u32)
        .position_centered()
        .build()
        .map_err(|e| e.to_string())?;
    let mut canvas = window
        .into_canvas()
        .present_vsync()
        .build()
        .map_err(|e| e.to_string())?;
    let texture_creator = canvas.texture_creator();
    let mut texture = texture_creator
        .create_texture_streaming(PixelFormatEnum::ABGR8888, WIDTH as u32, HEIGHT as u32)
        .map_err(|e| e.to_string())?;

    let audio = sdl.audio()?;
    let spec = AudioSpecDesired {
        freq: Some(44100),
        channels: Some(1),
        samples: Some(1024),
    };
    let queue: AudioQueue<f32> = audio.open_queue(None, &spec)?;
    queue.resume();

    let mut events = sdl.event_pump()?;
    let mut buttons: u8 = 0;
    let target_queued = (44100.0 * 0.06) as u32 * 4;

    'running: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                Event::KeyDown {
                    keycode: Some(k), ..
                } => match k {
                    Keycode::F5 => {
                        let _ = std::fs::write(&state_path, nes.save_state());
                    }
                    Keycode::R => nes.reset(),
                    Keycode::F7 => {
                        if let Ok(data) = std::fs::read(&state_path) {
                            nes.load_state(&data);
                        }
                    }
                    _ => {
                        for (key, bit) in BUTTONS {
                            if k == key {
                                buttons |= bit;
                            }
                        }
                    }
                },
                Event::KeyUp {
                    keycode: Some(k), ..
                } => {
                    for (key, bit) in BUTTONS {
                        if k == key {
                            buttons &= !bit;
                        }
                    }
                }
                _ => {}
            }
        }
        nes.set_buttons(0, buttons);
        while queue.size() < target_queued {
            nes.run_frame();
            queue.queue_audio(&nes.take_samples())?;
        }

        texture
            .update(None, pixel_bytes(nes.frame()), WIDTH * 4)
            .map_err(|e| e.to_string())?;
        canvas.clear();
        canvas.copy(&texture, None, None)?;
        canvas.present();
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn pixel_bytes(frame: &[u32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(frame.as_ptr() as *const u8, frame.len() * 4) }
}
