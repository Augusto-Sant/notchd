//! A grid view of a terminal (an `attach` with `render: grid`, experimental,
//! the `ghostty` feature): the daemon emulates the terminal with
//! libghostty-vt and sends the view the screen as styled rows, at most one
//! frame per [`FRAME`], only the rows that changed; the view sends keys,
//! which the daemon encodes for the program (legacy or the kitty keyboard
//! protocol, as the program asked). The UI draws text; it emulates nothing.
//!
//! libghostty's types are neither `Send` nor `Sync`, so the terminal lives on
//! one thread, fed through a channel: the program's output, the view's keys,
//! its size, the wheel.

use crate::hub::Hub;
use crate::ptyhost::{Out, Session};
use crate::router::err;
use libghostty_vt::alloc::{Allocator, Bytes};
use libghostty_vt::fmt::Format;
use libghostty_vt::key::{self, Action, Mods};
use libghostty_vt::kitty::graphics;
use libghostty_vt::mouse;
use libghostty_vt::paste;
use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RowIterator};
use libghostty_vt::screen::{CellWide, Screen};
use libghostty_vt::selection::FormatOptions;
use libghostty_vt::selection::gesture::{DragEvent, Geometry, Gesture, PressEvent, ReleaseEvent};
use libghostty_vt::style::{RgbColor, Underline};
use libghostty_vt::terminal::ScrollViewport;
use libghostty_vt::terminal::{Mode, Point, PointCoordinate};
use libghostty_vt::{RenderState, Terminal, TerminalOptions};
use notchd_proto::surface::{
    Attach, Grid, GridCursor, GridImage, GridLine, GridRun, KeyPress, MouseEv, SurfaceEv,
    SurfaceMsg, SurfaceRef, b64_decode,
};
use notchd_proto::{ToDaemon, ToUi, to_line};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// The shortest time between two frames.
const FRAME: Duration = Duration::from_millis(16);
/// Rows kept above the screen.
const SCROLLBACK: usize = 10_000;
/// A view that does not read for this long is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

enum In {
    Data(Vec<u8>),
    Title(String),
    Key(KeyPress),
    Input(Vec<u8>),
    Resize(u16, u16, Option<(u32, u32)>),
    Wheel(i32, Option<(f64, f64)>),
    Mouse(MouseEv),
    Paste(String),
    Close,
}

fn line(msg: &ToUi) -> Vec<u8> {
    let mut l = to_line(msg).into_bytes();
    l.push(b'\n');
    l
}

fn size(v: Option<u32>, or: u16) -> u16 {
    v.map_or(or, |n| n.clamp(2, 1000) as u16)
}

/// The daemon's end of a grid view: serves it until it or the program is done.
pub fn serve(hub: &Arc<Hub>, stream: UnixStream, mut reader: impl BufRead, req: &Attach) {
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    let attached = match hub.pty_attach(&req.r#ref, req.cols, req.rows) {
        Ok(a) => a,
        Err(e) => {
            let _ = out.write_all(&line(&err(None, "surface", e)));
            return;
        }
    };
    let session = Arc::clone(&attached.session);
    let sub = attached.att.sub;
    let (c0, r0) = session.size();
    let (cols, rows) = (size(req.cols, c0), size(req.rows, r0));
    let had_output = !attached.att.replay.is_empty();
    let (tx, rx) = sync_channel::<In>(256);

    // The program's output, from the session's queue to the terminal's thread.
    let _ = tx.send(In::Data(attached.att.replay));
    if !attached.att.title.is_empty() {
        let _ = tx.send(In::Title(attached.att.title));
    }
    let forward = {
        let tx = tx.clone();
        let exited = attached.att.exited;
        let att_rx = attached.att.rx;
        thread::spawn(move || {
            if !exited {
                for o in att_rx {
                    let msg = match o {
                        Out::Data(d) => In::Data(d),
                        Out::Title(t) => In::Title(t),
                        Out::Close => break,
                    };
                    if tx.send(msg).is_err() {
                        return;
                    }
                }
            }
            let _ = tx.send(In::Close);
        })
    };

    let term_thread = {
        let session = Arc::clone(&session);
        let rf = req.r#ref.clone();
        let colors = Colors::of(req);
        let px = cell_px(req.cw, req.ch);
        let stream = stream.try_clone();
        thread::spawn(move || {
            let _ = run(&rx, &mut out, &session, &rf, (cols, rows), px, &colors);
            let _ = out.write_all(&line(&ToUi::SurfaceClose(SurfaceRef { r#ref: rf })));
            if let Ok(s) = stream {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        })
    };

    // The program takes the view's size. At the size it already has, it is
    // made to draw again (one column narrower and back), as `attach` does:
    // what a replay that wrapped lost comes back.
    if session.size() == (cols, rows) {
        if had_output && !session.exited() {
            let s = Arc::clone(&session);
            thread::spawn(move || {
                s.resize(cols - 1, rows);
                thread::sleep(Duration::from_millis(60));
                s.resize(cols, rows);
            });
        }
    } else {
        session.resize(cols, rows);
    }

    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let Ok(ToDaemon::Surface(m)) = notchd_proto::parse::<ToDaemon>(buf.trim_end()) else {
            continue;
        };
        let msg = match m.ev {
            SurfaceEv::Key(k) => In::Key(k),
            SurfaceEv::Input { b64 } => match b64_decode(&b64) {
                Some(b) => In::Input(b),
                None => continue,
            },
            SurfaceEv::Resize { cols, rows, cw, ch } => {
                In::Resize(size(Some(cols), 80), size(Some(rows), 24), cell_px(cw, ch))
            }
            SurfaceEv::Wheel { rows, x, y } => In::Wheel(rows, x.zip(y)),
            SurfaceEv::Mouse(m) => In::Mouse(m),
            SurfaceEv::Paste { text } => In::Paste(text),
            _ => continue,
        };
        if tx.send(msg).is_err() {
            break;
        }
    }
    let _ = tx.send(In::Close);
    drop(tx);
    hub.pty_detach(&session, sub);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    let _ = term_thread.join();
    let _ = forward.join();
}

/// The terminal's thread.
fn run(
    rx: &std::sync::mpsc::Receiver<In>,
    out: &mut UnixStream,
    session: &Arc<Session>,
    rf: &str,
    (cols, rows): (u16, u16),
    px: Option<(u32, u32)>,
    colors: &Colors,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut term = Terminal::new(TerminalOptions {
        cols,
        rows,
        max_scrollback: SCROLLBACK,
    })?;
    colors.apply(&mut term)?;
    // A cell in the view's pixels, for what a program sizes in pixels.
    let mut cell = px.unwrap_or(CELL_PX);
    term.resize(cols, rows, cell.0, cell.1)?;
    // Kitty images: stored up to IMAGE_STORE, PNGs decoded here.
    term.set_kitty_image_storage_limit(IMAGE_STORE)?;
    let _ = graphics::set_png_decoder(Some(Box::new(PngDecoder)));
    let mut images = Images::default();
    // What the terminal answers (cursor position, device attributes, the
    // kitty keyboard flags) goes to the program.
    {
        let s = Arc::clone(session);
        term.on_pty_write(move |_t, data: &[u8]| s.input(data))?;
    }
    let mut frame = Frame::new()?;
    let mut keys = key::Encoder::new()?;
    let mut ev = key::Event::new()?;
    let mut pointer = Pointer::new()?;
    let mut sent_mouse = false;
    let mut select = Select::new()?;
    // What the program put on the clipboard (OSC 52), to send the view.
    let copies: Arc<Mutex<Vec<String>>> = Arc::default();
    {
        let copies = Arc::clone(&copies);
        term.on_clipboard_write(move |_t, w| {
            let text = w
                .contents()
                .find(|c| c.mime.starts_with("text/"))
                .or_else(|| w.contents().next())
                .map(|c| c.data.to_string());
            if let Some(t) = text {
                copies.lock().unwrap_or_else(|p| p.into_inner()).push(t);
            }
            Ok(())
        })?;
    }
    let mut full = true;
    let mut dirty = true;
    let mut last = Instant::now() - FRAME;
    loop {
        let wait = if dirty {
            FRAME.saturating_sub(last.elapsed())
        } else {
            Duration::from_secs(3600)
        };
        match rx.recv_timeout(wait) {
            Ok(In::Data(d)) => {
                term.vt_write(&d);
                dirty = true;
            }
            Ok(In::Title(t)) => {
                out.write_all(&line(&ToUi::Surface(SurfaceMsg {
                    r#ref: rf.to_string(),
                    ev: SurfaceEv::Title { text: t },
                })))?;
            }
            // Ctrl+Shift+C copies the selection, Ctrl+Shift+V pastes: the
            // terminal's, as in Ghostty; the program never sees them.
            Ok(In::Key(k)) if k.mods == 5 && k.key == "KeyC" => {
                if let Some(text) = selected_text(&term)? {
                    send(out, rf, SurfaceEv::Copy { text })?;
                }
            }
            Ok(In::Key(k)) if k.mods == 5 && k.key == "KeyV" => {
                send(out, rf, SurfaceEv::Clipboard {})?;
            }
            Ok(In::Key(k)) => {
                let bytes = encode(&mut keys, &mut ev, &term, &k);
                if !bytes.is_empty() {
                    session.input(&bytes);
                    // Typing brings the view back to the bottom and ends a
                    // selection.
                    term.scroll_viewport(ScrollViewport::Bottom);
                    term.set_selection(None)?;
                    dirty = true;
                }
            }
            Ok(In::Paste(text)) => {
                let bracketed = term.mode(Mode::BRACKETED_PASTE)?;
                let mut data = text.into_bytes();
                let mut buf = vec![0u8; data.len() + 16];
                let n = paste::encode(&mut data, bracketed, &mut buf)?;
                session.input(&buf[..n]);
                term.scroll_viewport(ScrollViewport::Bottom);
                term.set_selection(None)?;
                dirty = true;
            }
            Ok(In::Input(b)) => session.input(&b),
            Ok(In::Resize(c, r, px)) => {
                if (c, r) != (term.cols()?, term.rows()?) || px.is_some_and(|p| p != cell) {
                    cell = px.unwrap_or(cell);
                    term.resize(c, r, cell.0, cell.1)?;
                    session.resize(c, r);
                    full = true;
                    dirty = true;
                }
            }
            Ok(In::Mouse(m)) => {
                // The program's, when it asked for the mouse and Shift is
                // up; else (and until a selection that began is let go) it
                // selects.
                let program = term.is_mouse_tracking()? && m.mods & 1 == 0 && !select.down;
                if program {
                    let bytes = pointer.encode(&term, &m);
                    if !bytes.is_empty() {
                        session.input(&bytes);
                    }
                } else if select.pointer(&term, &m)? {
                    dirty = true;
                }
            }
            Ok(In::Wheel(n, at)) => {
                if term.is_mouse_tracking()? {
                    // The program asked for the mouse: the wheel is its
                    // buttons 4 (up) and 5 (down), a press a row.
                    let (x, y) = at.unwrap_or((0.0, 0.0));
                    let m = MouseEv {
                        action: "press".into(),
                        button: Some(if n < 0 { "up" } else { "down" }.into()),
                        x,
                        y,
                        mods: 0,
                    };
                    for _ in 0..n.unsigned_abs().min(50) {
                        let bytes = pointer.encode(&term, &m);
                        session.input(&bytes);
                    }
                } else if term.active_screen()? == Screen::Alternate {
                    // A full-screen program scrolls itself: arrow keys, as
                    // most terminals do for the alternate screen.
                    let code = if n < 0 { "ArrowUp" } else { "ArrowDown" };
                    for _ in 0..n.unsigned_abs().min(50) {
                        let k = KeyPress {
                            key: code.into(),
                            mods: 0,
                            text: None,
                        };
                        let bytes = encode(&mut keys, &mut ev, &term, &k);
                        session.input(&bytes);
                    }
                } else {
                    term.scroll_viewport(ScrollViewport::Delta(n as isize));
                    dirty = true;
                }
            }
            Ok(In::Close) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let copied: Vec<String> =
            std::mem::take(&mut *copies.lock().unwrap_or_else(|p| p.into_inner()));
        for text in copied {
            send(out, rf, SurfaceEv::Copy { text })?;
        }
        if dirty && last.elapsed() >= FRAME {
            let mut g = frame.build(&term, full)?;
            g.images = images.frame(&term, out, rf, full)?;
            if full
                || !g.lines.is_empty()
                || g.cursor.is_some()
                || g.mouse != sent_mouse
                || g.images.is_some()
            {
                sent_mouse = g.mouse;
                out.write_all(&line(&ToUi::Surface(SurfaceMsg {
                    r#ref: rf.to_string(),
                    ev: SurfaceEv::Grid(g),
                })))?;
            }
            last = Instant::now();
            dirty = false;
            full = false;
        }
    }
    if dirty {
        let g = frame.build(&term, full)?;
        out.write_all(&line(&ToUi::Surface(SurfaceMsg {
            r#ref: rf.to_string(),
            ev: SurfaceEv::Grid(g),
        })))?;
    }
    Ok(())
}

fn send(out: &mut UnixStream, rf: &str, ev: SurfaceEv) -> std::io::Result<()> {
    out.write_all(&line(&ToUi::Surface(SurfaceMsg {
        r#ref: rf.to_string(),
        ev,
    })))
}

/// The selected text, plain (soft wraps joined, trailing blanks gone).
fn selected_text(
    term: &Terminal<'static, 'static>,
) -> Result<Option<String>, libghostty_vt::Error> {
    let opts = FormatOptions::new()
        .with_emit_format(Format::Plain)
        .with_unwrap(true)
        .with_trim(true);
    Ok(term
        .format_selection_alloc(None, opts)?
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .filter(|t| !t.is_empty()))
}

/// Selecting with the pointer: libghostty's gesture (a drag selects; two
/// clicks a word, three a line), installed as the terminal's selection,
/// which the frames then draw.
struct Select {
    gesture: Gesture<'static>,
    press: PressEvent<'static>,
    drag: DragEvent<'static>,
    release: ReleaseEvent<'static>,
    since: Instant,
    /// The left button went down here and is not up yet.
    down: bool,
}

/// Two clicks closer than this in time are a double click.
const REPEAT: Duration = Duration::from_millis(500);

impl Select {
    fn new() -> Result<Self, libghostty_vt::Error> {
        Ok(Select {
            gesture: Gesture::new()?,
            press: PressEvent::new()?,
            drag: DragEvent::new()?,
            release: ReleaseEvent::new()?,
            since: Instant::now(),
            down: false,
        })
    }

    /// A pointer event; whether the selection may have changed.
    fn pointer(
        &mut self,
        term: &Terminal<'static, 'static>,
        m: &MouseEv,
    ) -> Result<bool, libghostty_vt::Error> {
        let left = m.button.as_deref() == Some("left");
        let (cols, rows) = (term.cols()?, term.rows()?);
        let cx = (m.x.max(0.0) as u16).min(cols.saturating_sub(1));
        let cy = (m.y.max(0.0) as u32).min(u32::from(rows.saturating_sub(1)));
        let at = || term.grid_ref(Point::Viewport(PointCoordinate { x: cx, y: cy }));
        let (cw, ch) = CELL_PX;
        let (px, py) = (m.x.max(0.0) * f64::from(cw), m.y.max(0.0) * f64::from(ch));
        match m.action.as_str() {
            "press" if left => {
                self.down = true;
                let sel = self
                    .press
                    .set_position(px, py)?
                    .set_time(self.since.elapsed())?
                    .set_repeat_interval(REPEAT)?
                    .set_repeat_distance(f64::from(cw))?
                    .apply(&mut self.gesture, term, at()?)?;
                term.set_selection(sel.as_ref())?;
                Ok(true)
            }
            "motion" if self.down => {
                let sel = self.drag.set_position(px, py)?.apply(
                    &mut self.gesture,
                    term,
                    at()?,
                    Geometry {
                        columns: u32::from(cols),
                        cell_width: cw,
                        padding_left: 0,
                        screen_height: u32::from(rows) * ch,
                    },
                )?;
                term.set_selection(sel.as_ref())?;
                Ok(true)
            }
            "release" if left && self.down => {
                self.down = false;
                self.release.apply(&mut self.gesture, term, Some(at()?))?;
                // A plain click selects nothing: it lets go of what was.
                if !self.gesture.dragged(term)? && self.gesture.click_count(term)? <= 1 {
                    term.set_selection(None)?;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

/// A cell's size in pixels, when the view said it.
fn cell_px(cw: Option<u32>, ch: Option<u32>) -> Option<(u32, u32)> {
    Some((cw?.clamp(1, 200), ch?.clamp(1, 400)))
}

/// What a terminal keeps of kitty images, at most.
const IMAGE_STORE: u64 = 64 << 20;
/// An image wider or taller than this goes to the view scaled down.
const IMAGE_MAX: u32 = 2048;

/// libghostty's PNG decoder: PNGs to 8-bit RGBA.
struct PngDecoder;

impl graphics::DecodePng for PngDecoder {
    fn decode_png<'alloc>(
        &mut self,
        alloc: &'alloc Allocator<'_>,
        data: &[u8],
    ) -> Option<graphics::DecodedImage<'alloc>> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
        decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0u8; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        let rgba = to_rgba(&buf[..info.buffer_size()], info.color_type)?;
        let mut bytes = Bytes::new_with_alloc(alloc, rgba.len()).ok()?;
        bytes.copy_from_slice(&rgba);
        Some(graphics::DecodedImage {
            width: info.width,
            height: info.height,
            data: bytes,
        })
    }
}

/// 8-bit pixels of a color type as RGBA.
fn to_rgba(px: &[u8], color: png::ColorType) -> Option<Vec<u8>> {
    Some(match color {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    })
}

/// An image's pixels as a PNG, scaled down (nearest) past IMAGE_MAX.
fn encode_png(img: &graphics::Image<'_>) -> Option<Vec<u8>> {
    if img.compression().ok()? != graphics::Compression::None {
        return None;
    }
    let (w, h) = (img.width().ok()?, img.height().ok()?);
    let data = img.data().ok()?;
    let rgba = match img.format().ok()? {
        graphics::ImageFormat::Rgba => to_rgba(data, png::ColorType::Rgba)?,
        graphics::ImageFormat::Rgb => to_rgba(data, png::ColorType::Rgb)?,
        graphics::ImageFormat::GrayAlpha => to_rgba(data, png::ColorType::GrayscaleAlpha)?,
        graphics::ImageFormat::Gray => to_rgba(data, png::ColorType::Grayscale)?,
        _ => return None,
    };
    if w == 0 || h == 0 || rgba.len() < (w * h * 4) as usize {
        return None;
    }
    let step = w.max(h).div_ceil(IMAGE_MAX).max(1);
    let (ow, oh) = (w.div_ceil(step), h.div_ceil(step));
    let pixels: Vec<u8> = if step == 1 {
        rgba
    } else {
        let mut out = Vec::with_capacity((ow * oh * 4) as usize);
        for y in 0..oh {
            for x in 0..ow {
                let i = (((y * step) * w + x * step) * 4) as usize;
                out.extend_from_slice(&rgba[i..i + 4]);
            }
        }
        out
    };
    let mut png_out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_out, ow, oh);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().ok()?;
        w.write_image_data(&pixels).ok()?;
    }
    Some(png_out)
}

/// The images the view was sent: each image's generation, and the last
/// placements.
#[derive(Default)]
struct Images {
    sent: HashMap<u32, u64>,
    last: Vec<GridImage>,
    placements: Option<graphics::PlacementIterator<'static>>,
}

impl Images {
    /// Sends the view the images it has not got yet; the placements when
    /// they changed (or `full`), else nothing.
    fn frame(
        &mut self,
        term: &Terminal<'static, 'static>,
        out: &mut UnixStream,
        rf: &str,
        full: bool,
    ) -> Result<Option<Vec<GridImage>>, Box<dyn std::error::Error>> {
        let now = self.placements(term)?;
        let g = term.kitty_graphics()?;
        for p in &now {
            let Some(img) = g.image(p.id) else { continue };
            let generation = img.generation().unwrap_or(0);
            if self.sent.get(&p.id) == Some(&generation) {
                continue;
            }
            self.sent.insert(p.id, generation);
            if let Some(bytes) = encode_png(&img) {
                send(
                    out,
                    rf,
                    SurfaceEv::Image {
                        id: p.id,
                        png: notchd_proto::surface::b64_encode(&bytes),
                    },
                )?;
            }
        }
        if !full && now == self.last {
            return Ok(None);
        }
        if full && now.is_empty() && self.last.is_empty() {
            return Ok(None);
        }
        self.last.clone_from(&now);
        Ok(Some(now))
    }

    /// The placements on the screen now.
    fn placements(
        &mut self,
        term: &Terminal<'static, 'static>,
    ) -> Result<Vec<GridImage>, libghostty_vt::Error> {
        let g = term.kitty_graphics()?;
        if self.placements.is_none() {
            self.placements = Some(graphics::PlacementIterator::new()?);
        }
        let it = self.placements.as_mut().expect("made above");
        let mut list = it.update(&g)?;
        let mut out = Vec::new();
        while let Some(p) = list.next() {
            if p.is_virtual().unwrap_or(true) {
                continue;
            }
            let id = p.image_id()?;
            let Some(img) = g.image(id) else { continue };
            let Some(pos) = p.viewport_pos(&img, term)? else {
                continue;
            };
            let size = p.pixel_size(&img, term)?;
            let src = p.source_rect(&img)?;
            out.push(GridImage {
                id,
                x: pos.col,
                y: pos.row,
                xo: p.x_offset()?,
                yo: p.y_offset()?,
                w: size.width,
                h: size.height,
                sx: src.x,
                sy: src.y,
                sw: src.width,
                sh: src.height,
                z: p.z()?,
            });
        }
        Ok(out)
    }
}

/// The size of a cell the mouse encoder is told: the view sends cells, and
/// a program that asks for pixels (SGR pixels) gets these.
const CELL_PX: (u32, u32) = (10, 20);

/// The mouse: libghostty's encoder, which writes what the program asked for
/// (X10, SGR, URXVT, SGR pixels; press only, buttons, any motion), and which
/// buttons are held.
struct Pointer {
    enc: mouse::Encoder<'static>,
    ev: mouse::Event<'static>,
    held: u8,
}

impl Pointer {
    fn new() -> Result<Self, libghostty_vt::Error> {
        Ok(Pointer {
            enc: mouse::Encoder::new()?,
            ev: mouse::Event::new()?,
            held: 0,
        })
    }

    /// The bytes a pointer event sends the program; none when it did not ask
    /// for that kind of event.
    fn encode(&mut self, term: &Terminal<'static, 'static>, m: &MouseEv) -> Vec<u8> {
        let action = match m.action.as_str() {
            "press" => mouse::Action::Press,
            "release" => mouse::Action::Release,
            _ => mouse::Action::Motion,
        };
        let button = m.button.as_deref().map(|b| match b {
            "left" => mouse::Button::Left,
            "middle" => mouse::Button::Middle,
            "right" => mouse::Button::Right,
            "up" => mouse::Button::Four,
            "down" => mouse::Button::Five,
            _ => mouse::Button::Unknown,
        });
        let bit = match button {
            Some(mouse::Button::Left) => 1,
            Some(mouse::Button::Middle) => 2,
            Some(mouse::Button::Right) => 4,
            _ => 0,
        };
        match action {
            mouse::Action::Press => self.held |= bit,
            mouse::Action::Release => self.held &= !bit,
            _ => {}
        }
        let (cols, rows) = (
            u32::from(term.cols().unwrap_or(1)),
            u32::from(term.rows().unwrap_or(1)),
        );
        let (cw, ch) = CELL_PX;
        self.enc
            .set_options_from_terminal(term)
            .set_size(mouse::EncoderSize {
                screen_width: cols * cw,
                screen_height: rows * ch,
                cell_width: cw,
                cell_height: ch,
                padding_top: 0,
                padding_bottom: 0,
                padding_right: 0,
                padding_left: 0,
            })
            .set_any_button_pressed(self.held != 0);
        self.ev
            .set_action(action)
            .set_button(button)
            .set_mods(mods_of(m.mods))
            .set_position(mouse::Position {
                x: (m.x.max(0.0) * f64::from(cw)) as f32,
                y: (m.y.max(0.0) * f64::from(ch)) as f32,
            });
        let mut v = Vec::new();
        let _ = self.enc.encode_to_vec(&self.ev, &mut v);
        v
    }
}

/// A key's or the mouse's `mods` bits as libghostty's.
fn mods_of(bits: u32) -> Mods {
    let mut mods = Mods::empty();
    for (bit, m) in [
        (1, Mods::SHIFT),
        (2, Mods::ALT),
        (4, Mods::CTRL),
        (8, Mods::SUPER),
    ] {
        if bits & bit != 0 {
            mods |= m;
        }
    }
    mods
}

/// The libghostty key a W3C `code` names (`KeyA`, `Enter`...).
fn key_named(name: &str) -> Option<key::Key> {
    static NAMES: OnceLock<HashMap<String, key::Key>> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        let mut m = HashMap::new();
        for n in 0..1024u32 {
            if let Ok(k) = key::Key::try_from(n) {
                let mut s = format!("{k:?}");
                // The crate names letters `A`; the W3C code is `KeyA`.
                if s.len() == 1 {
                    s = format!("Key{s}");
                }
                m.insert(s, k);
            }
        }
        m
    });
    names.get(name).copied()
}

/// The bytes a key sends the program, as the terminal's modes say.
fn encode(
    keys: &mut key::Encoder,
    ev: &mut key::Event,
    term: &Terminal<'static, 'static>,
    k: &KeyPress,
) -> Vec<u8> {
    let Some(code) = key_named(&k.key) else {
        // A key libghostty does not name: what it types, as it is.
        return k.text.clone().unwrap_or_default().into_bytes();
    };
    let mods = mods_of(k.mods);
    let unshifted = k
        .key
        .strip_prefix("Key")
        .or_else(|| k.key.strip_prefix("Digit"))
        .and_then(|s| s.chars().next())
        .map(|c| c.to_ascii_lowercase())
        .or((k.key == "Space").then_some(' '));
    ev.set_action(Action::Press)
        .set_key(code)
        .set_mods(mods)
        .set_consumed_mods(if k.text.is_some() {
            mods & Mods::SHIFT
        } else {
            Mods::empty()
        })
        .set_utf8(k.text.clone().filter(|t| !t.is_empty()));
    if let Some(c) = unshifted {
        ev.set_unshifted_codepoint(c);
    }
    keys.set_options_from_terminal(term);
    let mut v = Vec::new();
    let _ = keys.encode_to_vec(ev, &mut v);
    v
}

/// The view's theme: its default colors and its ANSI palette.
struct Colors {
    fg: Option<RgbColor>,
    bg: Option<RgbColor>,
    /// From color 0 on; an entry that is not `#rrggbb` keeps the
    /// terminal's own.
    palette: Vec<Option<RgbColor>>,
}

impl Colors {
    fn of(req: &Attach) -> Colors {
        Colors {
            fg: parse_hex(req.fg.as_deref()),
            bg: parse_hex(req.bg.as_deref()),
            palette: req
                .palette
                .iter()
                .flatten()
                .take(16)
                .map(|c| parse_hex(Some(c)))
                .collect(),
        }
    }

    fn apply(&self, term: &mut Terminal<'static, 'static>) -> Result<(), libghostty_vt::Error> {
        term.set_default_fg_color(self.fg)?
            .set_default_bg_color(self.bg)?;
        if self.palette.iter().any(Option::is_some) {
            let mut p = term.default_color_palette()?;
            for (i, c) in self.palette.iter().enumerate() {
                if let Some(c) = c {
                    p.0[i] = *c;
                }
            }
            term.set_default_color_palette(Some(p))?;
        }
        Ok(())
    }
}

/// `#rrggbb`, else nothing.
fn parse_hex(s: Option<&str>) -> Option<RgbColor> {
    let h = s?.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(RgbColor {
        r: (n >> 16) as u8,
        g: (n >> 8) as u8,
        b: n as u8,
    })
}

fn hex(c: RgbColor) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// What builds a frame: libghostty's render state and its iterators, kept
/// across frames (they track which rows changed).
struct Frame {
    rs: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
}

#[derive(PartialEq, Clone)]
struct Pen {
    fg: Option<String>,
    bg: Option<String>,
    a: u32,
}

impl Frame {
    fn new() -> Result<Self, libghostty_vt::Error> {
        Ok(Frame {
            rs: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
        })
    }

    fn build(
        &mut self,
        term: &Terminal<'static, 'static>,
        full: bool,
    ) -> Result<Grid, libghostty_vt::Error> {
        let snap = self.rs.update(term)?;
        let colors = snap.colors()?;
        let all = full || snap.dirty()? == Dirty::Full;
        let (fg0, bg0) = (hex(colors.foreground), hex(colors.background));
        let cursor = if snap.cursor_visible()? {
            let style = snap.cursor_visual_style()?;
            snap.cursor_viewport()?.map(|c| GridCursor {
                x: u32::from(c.x),
                y: u32::from(c.y),
                style: match style {
                    CursorVisualStyle::Bar => "bar",
                    CursorVisualStyle::Underline => "underline",
                    CursorVisualStyle::BlockHollow => "hollow",
                    _ => "block",
                }
                .into(),
            })
        } else {
            None
        };
        let mut lines = Vec::new();
        let mut it = self.rows.update(&snap)?;
        let mut y = 0u32;
        while let Some(row) = it.next() {
            if all || row.dirty()? {
                let mut runs: Vec<GridRun> = Vec::new();
                let mut cells = self.cells.update(row)?;
                let mut x = 0u32;
                while let Some(c) = cells.next() {
                    let wide = c.raw_cell()?.wide()?;
                    // The wide cell before already took this column.
                    if wide == CellWide::SpacerTail {
                        continue;
                    }
                    if wide == CellWide::SpacerHead {
                        x += 1;
                        continue;
                    }
                    let st = c.style()?;
                    let mut fg = c.fg_color()?.map(hex);
                    let mut bg = c.bg_color()?.map(hex);
                    if st.inverse {
                        let f = fg.take().unwrap_or_else(|| fg0.clone());
                        let b = bg.take().unwrap_or_else(|| bg0.clone());
                        fg = Some(b);
                        bg = Some(f);
                    }
                    let a = u32::from(st.bold)
                        | u32::from(st.italic) << 1
                        | u32::from(st.underline != Underline::None) << 2
                        | u32::from(st.strikethrough) << 3
                        | u32::from(st.faint) << 4
                        | u32::from(st.inverse) << 5;
                    let g = c.graphemes()?;
                    let text: String = if g.is_empty() || st.invisible {
                        " ".into()
                    } else {
                        g.into_iter().collect()
                    };
                    let w = if wide == CellWide::Wide { 2 } else { 1 };
                    let pen = Pen { fg, bg, a };
                    // A wide cell is a run of its own, so the UI places
                    // it on its two cells whatever the font draws.
                    let joins = w == 1
                        && runs.last().is_some_and(|r| {
                            r.w as usize == r.text.chars().count()
                                && r.x + r.w == x
                                && pen
                                    == Pen {
                                        fg: r.fg.clone(),
                                        bg: r.bg.clone(),
                                        a: r.a,
                                    }
                        });
                    if joins {
                        let r = runs.last_mut().expect("a run");
                        r.text.push_str(&text);
                        r.w += 1;
                    } else {
                        runs.push(GridRun {
                            x,
                            w,
                            text,
                            fg: pen.fg,
                            bg: pen.bg,
                            a: pen.a,
                        });
                    }
                    x += w;
                }
                // Trailing blanks of the default look are not sent.
                while runs
                    .last()
                    .is_some_and(|r| r.bg.is_none() && r.a & 4 == 0 && r.text.trim_end().is_empty())
                {
                    runs.pop();
                }
                if let Some(r) = runs.last_mut()
                    && r.bg.is_none()
                    && r.a & 4 == 0
                {
                    let t = r.text.trim_end().to_string();
                    let cut = r.text.chars().count() - t.chars().count();
                    if r.w as usize >= cut {
                        r.w -= cut as u32;
                    }
                    r.text = t;
                }
                let sel = row
                    .selection()?
                    .map(|s| [u32::from(s.start_x), u32::from(s.end_x)]);
                lines.push(GridLine { y, runs, sel });
                row.set_dirty(false)?;
            }
            y += 1;
        }
        snap.set_dirty(Dirty::Clean)?;
        Ok(Grid {
            cols: u32::from(term.cols()?),
            rows: u32::from(term.rows()?),
            full: all,
            mouse: term.is_mouse_tracking()?,
            images: None,
            fg: fg0,
            bg: bg0,
            cursor,
            lines,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(cols: u16, rows: u16) -> Terminal<'static, 'static> {
        Terminal::new(TerminalOptions {
            cols,
            rows,
            max_scrollback: 100,
        })
        .unwrap()
    }

    #[test]
    fn a_frame_is_the_screen_as_styled_runs_and_then_only_what_changed() {
        let mut t = term(20, 4);
        let mut f = Frame::new().unwrap();
        t.vt_write(b"\x1b[1;32mok\x1b[0m plain\r\n\x1b[2;5H\xf0\x9f\x98\x80x");
        let g = f.build(&t, true).unwrap();
        assert!(g.full);
        assert_eq!(g.lines.len(), 4);
        let r0 = &g.lines[0].runs;
        assert_eq!(
            (r0[0].x, r0[0].w, r0[0].text.as_str(), r0[0].a),
            (0, 2, "ok", 1)
        );
        assert!(r0[0].fg.is_some(), "green");
        assert_eq!(
            (r0[1].x, r0[1].text.as_str(), r0[1].fg.as_ref()),
            (2, " plain", None)
        );
        // The emoji takes two cells and is a run of its own; `x` after it.
        let r1 = &g.lines[1].runs;
        let emoji = r1.iter().find(|r| r.text == "😀").expect("the emoji");
        assert_eq!((emoji.x, emoji.w), (4, 2));
        assert_eq!(r1.last().unwrap().x, 6);
        assert!(g.lines[3].runs.is_empty(), "a blank row has no runs");
        assert_eq!(g.cursor.as_ref().map(|c| (c.x, c.y)), Some((7, 1)));

        t.vt_write(b"\x1b[4;1Hlast");
        let g = f.build(&t, false).unwrap();
        assert!(!g.full);
        let ys: Vec<u32> = g.lines.iter().map(|l| l.y).collect();
        assert!(
            ys.contains(&3) && !ys.contains(&0),
            "only the rows that changed: {ys:?}"
        );
    }

    #[test]
    fn keys_follow_the_kitty_protocol_once_the_program_asks_for_it() {
        let mut t = term(20, 4);
        let mut keys = key::Encoder::new().unwrap();
        let mut ev = key::Event::new().unwrap();
        let shift_enter = KeyPress {
            key: "Enter".into(),
            mods: 1,
            text: None,
        };
        let a = KeyPress {
            key: "KeyA".into(),
            mods: 0,
            text: Some("a".into()),
        };
        let ctrl_c = KeyPress {
            key: "KeyC".into(),
            mods: 4,
            text: None,
        };
        assert_eq!(encode(&mut keys, &mut ev, &t, &a), b"a");
        assert_eq!(encode(&mut keys, &mut ev, &t, &ctrl_c), b"\x03");
        // Legacy: as Ghostty sends it (modifyOtherKeys), not a bare \r.
        assert_eq!(
            encode(&mut keys, &mut ev, &t, &shift_enter),
            b"\x1b[27;2;13~"
        );
        // CSI > 1 u: disambiguate. Shift+Enter is its own key now.
        t.vt_write(b"\x1b[>1u");
        assert_eq!(encode(&mut keys, &mut ev, &t, &shift_enter), b"\x1b[13;2u");
        assert_eq!(encode(&mut keys, &mut ev, &t, &a), b"a");
        assert_eq!(encode(&mut keys, &mut ev, &t, &ctrl_c), b"\x1b[99;5u");
    }

    #[test]
    fn the_mouse_goes_to_a_program_that_asked_for_it_in_its_format() {
        let mut t = term(20, 4);
        let mut p = Pointer::new().unwrap();
        let click = |action: &str| MouseEv {
            action: action.into(),
            button: Some("left".into()),
            x: 3.5,
            y: 1.2,
            mods: 0,
        };
        assert!(p.encode(&t, &click("press")).is_empty(), "nobody asked");
        let mut f = Frame::new().unwrap();
        assert!(!f.build(&t, true).unwrap().mouse);
        // Button tracking (1002) in SGR (1006): cells from 1, as the
        // program counts them.
        t.vt_write(b"\x1b[?1002h\x1b[?1006h");
        assert!(f.build(&t, false).unwrap().mouse, "the frame says so");
        assert_eq!(p.encode(&t, &click("press")), b"\x1b[<0;4;2M");
        assert_eq!(p.encode(&t, &click("release")), b"\x1b[<0;4;2m");
        let wheel = MouseEv {
            action: "press".into(),
            button: Some("down".into()),
            x: 0.0,
            y: 0.0,
            mods: 0,
        };
        assert_eq!(p.encode(&t, &wheel), b"\x1b[<65;1;1M");
    }

    #[test]
    fn the_views_palette_colors_the_ansi_colors_and_the_rest_stay() {
        let Ok(ToDaemon::Attach(req)) = notchd_proto::parse::<ToDaemon>(
            r##"{"t":"attach","ref":"term:x","render":"grid","palette":["#000001","nope","#00ff00"]}"##,
        ) else {
            panic!("an attach")
        };
        let mut t = term(20, 2);
        let before = t.default_color_palette().unwrap();
        Colors::of(&req).apply(&mut t).unwrap();
        let p = t.color_palette().unwrap();
        assert_eq!(hex(p.0[0]), "#000001");
        assert_eq!(
            hex(p.0[1]),
            hex(before.0[1]),
            "an entry that is no color keeps the terminal's"
        );
        assert_eq!(hex(p.0[2]), "#00ff00");
        assert_eq!(hex(p.0[9]), hex(before.0[9]));
        // Text in color 2 is drawn in the theme's.
        t.vt_write(b"\x1b[32mok");
        let g = Frame::new().unwrap().build(&t, true).unwrap();
        assert_eq!(g.lines[0].runs[0].fg.as_deref(), Some("#00ff00"));
    }

    fn at(action: &str, x: f64, y: f64) -> MouseEv {
        MouseEv {
            action: action.into(),
            button: (action != "motion").then(|| "left".into()),
            x,
            y,
            mods: 0,
        }
    }

    #[test]
    fn a_drag_selects_the_frame_shows_it_and_the_copy_is_its_text() {
        let mut t = term(20, 3);
        t.vt_write(b"hello world\r\nsecond line");
        let mut s = Select::new().unwrap();
        let mut f = Frame::new().unwrap();
        f.build(&t, true).unwrap();
        s.pointer(&t, &at("press", 0.2, 0.5)).unwrap();
        s.pointer(&t, &at("motion", 4.6, 0.5)).unwrap();
        s.pointer(&t, &at("release", 4.6, 0.5)).unwrap();
        assert_eq!(selected_text(&t).unwrap().as_deref(), Some("hello"));
        let g = f.build(&t, false).unwrap();
        let row0 = g.lines.iter().find(|l| l.y == 0).expect("row 0 changed");
        assert_eq!(row0.sel, Some([0, 4]));
        // Across rows: the line end is a line end.
        s.pointer(&t, &at("press", 6.2, 0.5)).unwrap();
        s.pointer(&t, &at("motion", 5.6, 1.5)).unwrap();
        s.pointer(&t, &at("release", 5.6, 1.5)).unwrap();
        assert_eq!(selected_text(&t).unwrap().as_deref(), Some("world\nsecond"));
        // A plain click lets go of it.
        s.pointer(&t, &at("press", 2.0, 2.5)).unwrap();
        s.pointer(&t, &at("release", 2.0, 2.5)).unwrap();
        assert_eq!(selected_text(&t).unwrap(), None);
    }

    #[test]
    fn two_clicks_select_a_word_and_three_the_line() {
        let mut t = term(30, 2);
        t.vt_write(b"git commit --amend");
        let mut s = Select::new().unwrap();
        for _ in 0..2 {
            s.pointer(&t, &at("press", 6.5, 0.5)).unwrap();
            s.pointer(&t, &at("release", 6.5, 0.5)).unwrap();
        }
        assert_eq!(selected_text(&t).unwrap().as_deref(), Some("commit"));
        s.pointer(&t, &at("press", 6.5, 0.5)).unwrap();
        s.pointer(&t, &at("release", 6.5, 0.5)).unwrap();
        assert_eq!(
            selected_text(&t).unwrap().as_deref(),
            Some("git commit --amend")
        );
    }

    #[test]
    fn what_the_program_copies_with_osc_52_is_heard() {
        let mut t = term(20, 2);
        let got: Arc<Mutex<Vec<String>>> = Arc::default();
        let g = Arc::clone(&got);
        t.on_clipboard_write(move |_t, w| {
            g.lock()
                .unwrap()
                .extend(w.contents().map(|c| c.data.to_string()));
            Ok(())
        })
        .unwrap();
        // "copied" in base64.
        t.vt_write(b"\x1b]52;c;Y29waWVk\x07");
        assert_eq!(*got.lock().unwrap(), vec!["copied".to_string()]);
    }

    #[test]
    fn a_paste_is_bracketed_when_the_program_asked_and_never_carries_escapes() {
        let mut data = b"ls\x1b[201~\nrm".to_vec();
        let mut buf = vec![0u8; data.len() + 16];
        let n = paste::encode(&mut data, true, &mut buf).unwrap();
        let out = &buf[..n];
        assert!(out.starts_with(b"\x1b[200~") && out.ends_with(b"\x1b[201~"));
        assert_eq!(
            out.windows(6).filter(|w| *w == &b"\x1b[201~"[..]).count(),
            1,
            "{out:?}"
        );
        let mut data = b"a\nb".to_vec();
        let mut buf = vec![0u8; data.len() + 16];
        let n = paste::encode(&mut data, false, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"a\rb", "unbracketed, a line end is Enter");
    }

    /// A grid terminal as `run` makes one: cells of 10x20 px, kitty images
    /// on, PNGs decoded.
    fn image_term() -> Terminal<'static, 'static> {
        let mut t = term(20, 5);
        t.resize(20, 5, 10, 20).unwrap();
        t.set_kitty_image_storage_limit(IMAGE_STORE).unwrap();
        graphics::set_png_decoder(Some(Box::new(PngDecoder))).unwrap();
        t
    }

    /// What a frame sent: the placements, and each `image` event decoded,
    /// (id, width, height).
    type Sent = (Option<Vec<GridImage>>, Vec<(u32, u32, u32)>);

    /// The `image` events a frame sent, decoded: (id, width, height).
    fn sent_images(t: &Terminal<'static, 'static>, imgs: &mut Images, full: bool) -> Sent {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        let placed = imgs.frame(t, &mut a, "term:x", full).unwrap();
        drop(a);
        let mut text = String::new();
        std::io::Read::read_to_string(&mut b, &mut text).unwrap();
        let mut got = Vec::new();
        for l in text.lines() {
            if let Ok(ToUi::Surface(SurfaceMsg {
                ev: SurfaceEv::Image { id, png },
                ..
            })) = notchd_proto::parse::<ToUi>(l)
            {
                let bytes = notchd_proto::surface::b64_decode(&png).unwrap();
                let r = png::Decoder::new(std::io::Cursor::new(bytes))
                    .read_info()
                    .unwrap();
                got.push((id, r.info().width, r.info().height));
            }
        }
        (placed, got)
    }

    #[test]
    fn a_kitty_image_is_sent_once_as_a_png_and_placed_where_the_cursor_was() {
        let mut t = image_term();
        let mut imgs = Images::default();
        // 2x2 RGBA pixels (f=32), id 7, put at the cursor (row 1, col 3).
        let px: Vec<u8> = (0..16)
            .map(|i| if i % 4 == 3 { 255 } else { 200 })
            .collect();
        let b64 = notchd_proto::surface::b64_encode(&px);
        t.vt_write(format!("\x1b[2;4H\x1b_Ga=T,f=32,s=2,v=2,i=7;{b64}\x1b\\").as_bytes());
        let (placed, got) = sent_images(&t, &mut imgs, false);
        assert_eq!(got, vec![(7, 2, 2)], "the image, once, as a PNG");
        let p = placed.expect("the placements changed");
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].id, p[0].x, p[0].y, p[0].sw, p[0].sh), (7, 3, 1, 2, 2));
        // Nothing changed: nothing sent.
        let (placed, got) = sent_images(&t, &mut imgs, false);
        assert_eq!((placed, got.len()), (None, 0));
        // Deleted: the placements go empty.
        t.vt_write(b"\x1b_Ga=d,d=A\x1b\\");
        let (placed, _) = sent_images(&t, &mut imgs, false);
        assert_eq!(placed, Some(vec![]));
    }

    #[test]
    fn a_png_the_program_sends_is_decoded() {
        let mut t = image_term();
        let mut imgs = Images::default();
        // A 1x1 red PNG (f=100).
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
        t.vt_write(format!("\x1b_Ga=T,f=100,i=3;{png}\x1b\\").as_bytes());
        let (placed, got) = sent_images(&t, &mut imgs, false);
        assert_eq!(got, vec![(3, 1, 1)]);
        assert_eq!(placed.map(|p| p.len()), Some(1));
    }

    #[test]
    fn an_indexed_png_a_palette_image_decodes_to_rgba() {
        // 2x1, two palette colors: what theme previews and many PNGs are.
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, 2, 1);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(vec![255, 0, 0, 0, 0, 255]);
            enc.write_header()
                .unwrap()
                .write_image_data(&[0, 1])
                .unwrap();
        }
        let alloc_png = notchd_proto::surface::b64_encode(&bytes);
        let mut t = image_term();
        let mut imgs = Images::default();
        t.vt_write(format!("\x1b_Ga=T,f=100,i=9;{alloc_png}\x1b\\").as_bytes());
        let (placed, got) = sent_images(&t, &mut imgs, false);
        assert_eq!(got, vec![(9, 2, 1)], "decoded and sent");
        assert_eq!(placed.map(|p| p.len()), Some(1));
    }

    #[test]
    fn every_w3c_code_the_ui_sends_names_a_key() {
        for code in [
            "KeyA",
            "KeyZ",
            "Digit0",
            "Enter",
            "Escape",
            "Backspace",
            "Tab",
            "Space",
            "ArrowUp",
            "ArrowLeft",
            "Home",
            "End",
            "PageUp",
            "Delete",
            "F1",
            "F12",
            "Minus",
            "Slash",
        ] {
            assert!(key_named(code).is_some(), "{code}");
        }
    }
}
