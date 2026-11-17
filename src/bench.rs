//! In-process scroll benchmark (NANOFM_BENCH_SCROLL). Scrolls the active file list once per
//! drawn frame and records frame intervals and main-thread CPU time per frame. No OS input.
//!
//! Modes (NANOFM_BENCH_MODE):
//! - `notify` (default, every build): move the list's scroll offset and notify the view that
//!   renders it, which is exactly what gpui 0.2.2's wheel handler does.
//! - `wheel` (gpui-fast build only): feed a scroll event through `Window::dispatch_event`, the
//!   real wheel path including gpui-fast's scroll layers. gpui 0.2.2 keeps that API's
//!   result type private, so it cannot be called there.

use crate::ui::file_list::FileList;
use gpui::{App, Entity, Pixels, Point, Window, point, px};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub struct BenchConfig {
    pub warmup: usize,
    pub frames: usize,
    pub step: f32,
    pub position: Point<Pixels>,
    pub wheel: bool,
}

impl BenchConfig {
    /// `NANOFM_BENCH_SCROLL=<frames>[,<px per frame>]`
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("NANOFM_BENCH_SCROLL").ok()?;
        let mut it = raw.split(',');
        let frames = it.next()?.trim().parse().ok()?;
        let step = it.next().and_then(|s| s.trim().parse().ok()).unwrap_or(80.0);
        let wheel = std::env::var("NANOFM_BENCH_MODE").is_ok_and(|m| m == "wheel");
        Some(Self { warmup: 30, frames, step, position: point(px(600.), px(400.)), wheel })
    }
}

struct State {
    cfg: BenchConfig,
    list: Entity<FileList>,
    ticks: usize,
    samples: Vec<(Instant, Duration)>,
    proc_start: Option<(Instant, Duration)>,
}

fn thread_cpu() -> Duration {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

fn process_cpu() -> Duration {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let tv = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

pub fn start(cfg: BenchConfig, list: Entity<FileList>, window: &mut Window, cx: &mut App) {
    if cfg.wheel {
        wheel::hover(cfg.position, window, cx);
    }
    let state = Rc::new(RefCell::new(State { cfg, list, ticks: 0, samples: Vec::new(), proc_start: None }));
    tick(state, window, cx);
}

fn tick(state: Rc<RefCell<State>>, window: &mut Window, cx: &mut App) {
    let (cfg, list, done) = {
        let mut s = state.borrow_mut();
        s.ticks += 1;
        if s.ticks > s.cfg.warmup {
            if s.proc_start.is_none() {
                s.proc_start = Some((Instant::now(), process_cpu()));
            }
            s.samples.push((Instant::now(), thread_cpu()));
        }
        (s.cfg, s.list.clone(), s.samples.len() > s.cfg.frames)
    };
    if done {
        report(&state.borrow());
        cx.quit();
        return;
    }
    if cfg.wheel {
        wheel::scroll(cfg.position, cfg.step, window, cx);
    } else {
        let handle = list.read(cx).scroll_handle();
        let base = handle.0.borrow().base_handle.clone();
        let offset = base.offset();
        base.set_offset(point(offset.x, offset.y - px(cfg.step)));
        cx.notify(list.entity_id());
    }
    let next = state.clone();
    window.on_next_frame(move |window, cx| tick(next, window, cx));
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn report(s: &State) {
    let (t0, p0) = s.proc_start.unwrap();
    let wall = s.samples.last().unwrap().0 - t0;
    let proc_cpu = process_cpu() - p0;
    let mut iv: Vec<f64> = s.samples.windows(2).map(|w| (w[1].0 - w[0].0).as_secs_f64() * 1e3).collect();
    let mut cpu: Vec<f64> = s.samples.windows(2).map(|w| (w[1].1 - w[0].1).as_secs_f64() * 1e3).collect();
    let n = iv.len();
    iv.sort_by(|a, b| a.partial_cmp(b).unwrap());
    cpu.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let med = pct(&iv, 0.5);
    let slow = iv.iter().filter(|&&x| x > med * 1.5).count();
    let mean_cpu = cpu.iter().sum::<f64>() / n as f64;
    println!(
        "BENCH mode={} frames={n} step={}px wall={:.2}s fps={:.1} interval_ms p50={:.2} p95={:.2} p99={:.2} max={:.2} slow_frames={slow} main_cpu_ms p50={:.2} p95={:.2} mean={:.2} process_cpu={:.0}%",
        if s.cfg.wheel { "wheel" } else { "notify" },
        s.cfg.step,
        wall.as_secs_f64(),
        n as f64 / wall.as_secs_f64(),
        med,
        pct(&iv, 0.95),
        pct(&iv, 0.99),
        iv[n - 1],
        pct(&cpu, 0.5),
        pct(&cpu, 0.95),
        mean_cpu,
        proc_cpu.as_secs_f64() / wall.as_secs_f64() * 100.0,
    );
}

#[cfg(feature = "gpui-fast")]
mod wheel {
    use gpui::{App, InputEvent, MouseMoveEvent, Pixels, Point, ScrollDelta, ScrollEvent, TouchPhase, Window, point, px};

    pub fn hover(position: Point<Pixels>, window: &mut Window, cx: &mut App) {
        window.dispatch_event(MouseMoveEvent { position, ..Default::default() }.to_platform_input(), cx);
    }

    pub fn scroll(position: Point<Pixels>, step: f32, window: &mut Window, cx: &mut App) {
        let event = ScrollEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(-step))),
            touch_phase: TouchPhase::Moved,
            ..Default::default()
        };
        window.dispatch_event(event.to_platform_input(), cx);
    }
}

#[cfg(not(feature = "gpui-fast"))]
mod wheel {
    use gpui::{App, Pixels, Point, Window};

    pub fn hover(_: Point<Pixels>, _: &mut Window, _: &mut App) {
        panic!("NANOFM_BENCH_MODE=wheel needs the gpui-fast build");
    }

    pub fn scroll(_: Point<Pixels>, _: f32, _: &mut Window, _: &mut App) {
        unreachable!()
    }
}
