mod http;
mod png;

use http::{read_request, respond, Request};
use nes_core::nes::{HEIGHT, WIDTH};
use nes_core::Nes;
use std::net::{TcpListener, TcpStream};

const INDEX: &str = "nes-agent endpoints:\n\
POST /rom                              body: iNES file\n\
POST /step?frames=N&pad0=HH&pad1=HH    hold buttons, run N frames\n\
POST /reset\n\
GET  /screen                           PNG of current frame\n\
GET  /pixels                           raw RGBA bytes, 256x240\n\
GET  /mem?addr=HHHH&len=N              RAM/cartridge bytes, no side effects\n\
GET  /state                            save-state blob\n\
POST /state                            body: save-state blob\n\
GET  /status                           JSON frame counter and frame hash\n\
buttons bitmask: 01=A 02=B 04=select 08=start 10=up 20=down 40=left 80=right\n";

struct Server {
    nes: Option<Nes>,
    frame: u64,
}

fn main() {
    let mut port = 5555u16;
    let mut nes = None;
    for arg in std::env::args().skip(1) {
        if let Ok(p) = arg.parse() {
            port = p;
        } else {
            let rom = std::fs::read(&arg).unwrap_or_else(|e| fail(&arg, &e.to_string()));
            nes = Some(Nes::new(&rom).unwrap_or_else(|e| fail(&arg, &e)));
            println!("loaded {arg}");
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("bind 127.0.0.1:{port}: {e}");
        std::process::exit(1);
    });
    println!("nes-agent listening on http://127.0.0.1:{port}");
    let mut server = Server { nes, frame: 0 };
    for stream in listener.incoming().flatten() {
        handle(&mut server, stream);
    }
}

fn handle(server: &mut Server, mut stream: TcpStream) {
    let Some(req) = read_request(&mut stream) else {
        return;
    };
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => respond(&mut stream, 200, "text/plain", INDEX.as_bytes()),
        ("POST", "/rom") => match Nes::new(&req.body) {
            Ok(nes) => {
                server.nes = Some(nes);
                server.frame = 0;
                ok(&mut stream, server.frame)
            }
            Err(e) => respond(&mut stream, 400, "text/plain", e.as_bytes()),
        },
        ("POST", "/step") => with_nes(server, &mut stream, |server, stream| {
            let frames: u32 = parse(&req, "frames", 10, 1).min(36000);
            let pad0 = parse(&req, "pad0", 16, 0) as u8;
            let pad1 = parse(&req, "pad1", 16, 0) as u8;
            let nes = server.nes.as_mut().unwrap();
            nes.set_buttons(0, pad0);
            nes.set_buttons(1, pad1);
            for _ in 0..frames {
                nes.run_frame();
                nes.take_samples();
            }
            server.frame += frames as u64;
            ok(stream, server.frame)
        }),
        ("POST", "/reset") => with_nes(server, &mut stream, |server, stream| {
            server.nes.as_mut().unwrap().reset();
            ok(stream, server.frame)
        }),
        ("GET", "/screen") => with_nes(server, &mut stream, |server, stream| {
            let frame = server.nes.as_ref().unwrap().frame();
            let image = png::encode(WIDTH as u32, HEIGHT as u32, frame);
            respond(stream, 200, "image/png", &image)
        }),
        ("GET", "/pixels") => with_nes(server, &mut stream, |server, stream| {
            let frame = server.nes.as_ref().unwrap().frame();
            let bytes: Vec<u8> = frame.iter().flat_map(|px| px.to_le_bytes()).collect();
            respond(stream, 200, "application/octet-stream", &bytes)
        }),
        ("GET", "/mem") => with_nes(server, &mut stream, |server, stream| {
            let addr = parse(&req, "addr", 16, 0);
            let len = parse(&req, "len", 10, 1).min(0x10000);
            let nes = server.nes.as_mut().unwrap();
            let bytes: Vec<u8> = (0..len).map(|i| peek(nes, (addr + i) as u16)).collect();
            respond(stream, 200, "application/octet-stream", &bytes)
        }),
        ("GET", "/state") => with_nes(server, &mut stream, |server, stream| {
            let blob = server.nes.as_ref().unwrap().save_state();
            respond(stream, 200, "application/octet-stream", &blob)
        }),
        ("POST", "/state") => with_nes(server, &mut stream, |server, stream| {
            if server.nes.as_mut().unwrap().load_state(&req.body) {
                ok(stream, server.frame)
            } else {
                respond(stream, 400, "text/plain", b"bad state blob")
            }
        }),
        ("GET", "/status") => with_nes(server, &mut stream, |server, stream| {
            let hash = fnv1a(server.nes.as_ref().unwrap().frame());
            let body = format!("{{\"frame\":{},\"hash\":\"{hash:016x}\"}}", server.frame);
            respond(stream, 200, "application/json", body.as_bytes())
        }),
        _ => respond(&mut stream, 404, "text/plain", b"not found"),
    }
}

fn with_nes(
    server: &mut Server,
    stream: &mut TcpStream,
    action: impl FnOnce(&mut Server, &mut TcpStream),
) {
    if server.nes.is_some() {
        action(server, stream);
    } else {
        respond(stream, 409, "text/plain", b"no rom loaded");
    }
}

fn parse(req: &Request, key: &str, radix: u32, default: u32) -> u32 {
    req.param(key)
        .and_then(|v| u32::from_str_radix(v, radix).ok())
        .unwrap_or(default)
}

fn peek(nes: &mut Nes, addr: u16) -> u8 {
    match addr {
        0x0000..=0x1FFF => nes.m.ram[addr as usize & 0x07FF],
        0x6000..=0xFFFF => nes.m.cart.cpu_read(addr),
        _ => 0,
    }
}

fn ok(stream: &mut TcpStream, frame: u64) {
    let body = format!("{{\"ok\":true,\"frame\":{frame}}}");
    respond(stream, 200, "application/json", body.as_bytes());
}

fn fail(arg: &str, err: &str) -> ! {
    eprintln!("{arg}: {err}");
    std::process::exit(1);
}

fn fnv1a(frame: &[u32]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for px in frame {
        for b in px.to_le_bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}
