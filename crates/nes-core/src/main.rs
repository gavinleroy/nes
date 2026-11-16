use nes_core::nes::{Nes, HEIGHT, WIDTH};
use std::io::Write;

fn main() {
    let mut args = std::env::args().skip(1);
    let rom_path = args.next().unwrap_or_else(|| usage());
    let frames: u32 = args
        .next()
        .map(|s| s.parse().unwrap_or_else(|_| usage()))
        .unwrap_or(60);
    let out_path = args.next();
    let rom = std::fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("{rom_path}: {e}");
        std::process::exit(1);
    });
    let mut nes = Nes::new(&rom).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    for _ in 0..frames {
        nes.run_frame();
    }
    if let Some(path) = out_path {
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "P6 {WIDTH} {HEIGHT} 255").unwrap();
        for px in nes.frame() {
            f.write_all(&[
                (*px & 0xFF) as u8,
                (*px >> 8 & 0xFF) as u8,
                (*px >> 16 & 0xFF) as u8,
            ])
            .unwrap();
        }
    }
    let mut hash: u64 = 0xcbf29ce484222325;
    for px in nes.frame() {
        for b in px.to_le_bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    println!("{frames} frames, framebuffer fnv1a {hash:016x}");
}

fn usage() -> ! {
    eprintln!("usage: nes <rom.nes> [frames] [out.ppm]");
    std::process::exit(2);
}
