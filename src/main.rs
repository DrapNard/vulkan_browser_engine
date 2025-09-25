//! Vulkan Browser Engine Main Application
//!
//! Single-runtime, single-thread design: we construct ONE Tokio current-thread
//! runtime in `main()` and pass `&Runtime` to places that need async calls.
//! No nested runtimes, no `block_on` inside another runtime.

use std::env;
use std::rc::Rc;
use std::time::Instant;

use tokio::{runtime::{Builder, Runtime}, signal};
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use vulkan_browser_engine::{BrowserConfig, BrowserEngine};

use winit::{
    dpi::LogicalSize,
    event::{ElementState, Event, KeyEvent, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Fullscreen, WindowBuilder},
};

#[derive(Debug, Clone)]
struct AppConfig {
    url: Option<String>,
    headless: bool,
    benchmark: bool,
    enable_tracy: bool,
    log_level: Level,
    profile_startup: bool,
}

impl AppConfig {
    fn from_args() -> Self {
        let args: Vec<String> = env::args().collect();
        let mut config = Self::default();

        let mut i = 1_usize;
        while i < args.len() {
            match args[i].as_str() {
                "--url" => {
                    if i + 1 < args.len() {
                        config.url = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "--headless" => config.headless = true,
                "--benchmark" => config.benchmark = true,
                "--tracy" => config.enable_tracy = true,
                "--debug" => config.log_level = Level::DEBUG,
                "--trace" => config.log_level = Level::TRACE,
                "--profile" => config.profile_startup = true,
                _ => {}
            }
            i += 1;
        }

        config
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            url: None,
            headless: false,
            benchmark: false,
            enable_tracy: false,
            log_level: Level::INFO,
            profile_startup: false,
        }
    }
}

/// Headless navigation benchmark (uses the provided runtime)
async fn run_headless_benchmark(engine: &BrowserEngine) -> vulkan_browser_engine::Result<()> {
    let urls = vec![
        "https://example.com",
        "https://google.com",
        "https://github.com",
        "https://stackoverflow.com",
        "https://reddit.com",
    ];

    let start = Instant::now();

    for url in urls {
        let load_start = Instant::now();
        engine.load_url(url).await?;
        let load_time = load_start.elapsed();
        println!("Loaded {} in {:?}", url, load_time);

        let metrics = engine.get_performance_metrics().await;
        println!("Metrics: {}", serde_json::to_string_pretty(&metrics).unwrap());
    }

    println!("Total benchmark time: {:?}", start.elapsed());
    Ok(())
}

fn setup_logging(level: Level, enable_tracy: bool) {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");

    if enable_tracy {
        #[cfg(feature = "tracy")]
        tracy_client::Client::start();
    }
}

/// Manages application performance and resource monitoring
struct PerformanceMonitor {
    frame_count: u64,
    last_fps_log: Instant,
    total_render_time: std::time::Duration,
    slow_frame_count: u64,
}

impl PerformanceMonitor {
    fn new() -> Self {
        Self {
            frame_count: 0,
            last_fps_log: Instant::now(),
            total_render_time: std::time::Duration::ZERO,
            slow_frame_count: 0,
        }
    }

    fn record_frame(&mut self, frame_time: std::time::Duration) {
        self.frame_count += 1;
        self.total_render_time += frame_time;

        if frame_time.as_millis() > 16 {
            self.slow_frame_count += 1;
        }

        // Log performance stats every ~5 seconds
        if self.last_fps_log.elapsed().as_secs() >= 5 {
            let avg_frame_time =
                self.total_render_time.as_millis() as f64 / self.frame_count as f64;
            let fps = self.frame_count as f64 / self.last_fps_log.elapsed().as_secs_f64();
            let slow_frame_percentage =
                (self.slow_frame_count as f64 / self.frame_count as f64) * 100.0;

            info!(
                "Performance: {:.1} FPS, {:.2}ms avg frame time, {:.1}% slow frames",
                fps, avg_frame_time, slow_frame_percentage
            );

            // Reset counters
            self.frame_count = 0;
            self.total_render_time = std::time::Duration::ZERO;
            self.slow_frame_count = 0;
            self.last_fps_log = Instant::now();
        }
    }
}

/// Executes a tiny placeholder rendering workload to keep the loop ticking.
/// Replace with real engine-driven pipeline once available.
fn do_render_tick() -> std::time::Duration {
    let t0 = Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(1));
    t0.elapsed()
}

fn setup_signal_handlers(rt: &Runtime) {
    // Fire-and-forget task on the same runtime
    let _ = rt.spawn(async {
        if let Ok(()) = signal::ctrl_c().await {
            info!("Received SIGINT, shutting down gracefully");
            std::process::exit(0);
        }
    });
}

async fn handle_window_resize(
    engine: &BrowserEngine,
    new_w: u32,
    new_h: u32,
) -> vulkan_browser_engine::Result<()> {
    info!("Window resized to {}x{}", new_w, new_h);
    engine.resize_viewport(new_w, new_h).await?;
    Ok(())
}

#[allow(dead_code)]
async fn execute_render_frame(
    _engine: &BrowserEngine,
    start_time: Instant,
) -> Result<std::time::Duration, vulkan_browser_engine::BrowserError> {
    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    Ok(start_time.elapsed())
}

fn run_windowed(app_config: AppConfig, rt: &Runtime) -> vulkan_browser_engine::Result<()> {
    let event_loop = EventLoop::new().expect("Failed to create event loop");

    let window = WindowBuilder::new()
        .with_title("Vulkan Browser Engine")
        .with_inner_size(LogicalSize::new(1920, 1080))
        .build(&event_loop)
        .expect("Failed to create window");
    let window = Rc::new(window);

    let browser_config = BrowserConfig {
        enable_jit: true,
        enable_gpu_acceleration: true,
        enable_sandbox: true,
        enable_pwa: true,
        enable_chrome_apis: true,
        viewport_width: 1920,
        viewport_height: 1080,
        ..Default::default()
    };

    // Engine lives on this thread only.
    let engine = Rc::new(rt.block_on(BrowserEngine::new(browser_config))?);

    // Initial navigation
    if let Some(url) = app_config.url {
        rt.block_on(engine.load_url(&url))?;
    } else {
        rt.block_on(engine.load_url(
            "data:text/html,<html><body><h1>Vulkan Browser Engine</h1><p>Ready for navigation</p></body></html>",
        ))?;
    }

    // Performance counters
    let mut last_frame_time = Instant::now();
    let mut frame_count: u64 = 0;
    let mut is_fullscreen = false;
    let mut perf_monitor = PerformanceMonitor::new();

    // Capture in the closure (Rc clones are cheap and single-threaded).
    let window_for_loop = Rc::clone(&window);
    let engine_for_loop = Rc::clone(&engine);

    event_loop
        .run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::Poll);

            match event {
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => {
                    info!("Close requested, shutting down gracefully...");
                    if let Err(e) = rt.block_on(async { engine_for_loop.shutdown().await }) {
                        error!("Error during shutdown: {}", e);
                    }
                    elwt.exit();
                }

                Event::WindowEvent {
                    event: WindowEvent::Resized(size),
                    ..
                } => {
                    if let Err(e) = rt.block_on(handle_window_resize(
                        &engine_for_loop,
                        size.width,
                        size.height,
                    )) {
                        error!("Resize failed: {}", e);
                    }
                }

                Event::WindowEvent {
                    event:
                        WindowEvent::KeyboardInput {
                            event:
                                KeyEvent {
                                    physical_key: PhysicalKey::Code(keycode),
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        },
                    ..
                } => {
                    match keycode {
                        KeyCode::F5 | KeyCode::KeyR => {
                            if let Err(e) = rt.block_on(async { engine_for_loop.reload().await }) {
                                error!("Failed to reload: {}", e);
                            } else {
                                info!("Reloaded page");
                            }
                        }
                        KeyCode::F11 => {
                            is_fullscreen = !is_fullscreen;
                            if is_fullscreen {
                                let monitor = window_for_loop
                                    .current_monitor()
                                    .or_else(|| window_for_loop.available_monitors().next());
                                window_for_loop
                                    .set_fullscreen(Some(Fullscreen::Borderless(monitor)));
                                info!("Entered fullscreen mode");
                            } else {
                                window_for_loop.set_fullscreen(None);
                                info!("Exited fullscreen mode");
                            }
                        }
                        KeyCode::F12 => {
                            info!("Developer tools toggle requested (engine API required)");
                        }
                        KeyCode::Escape => {
                            if is_fullscreen {
                                is_fullscreen = false;
                                window_for_loop.set_fullscreen(None);
                                info!("Exited fullscreen mode with Escape");
                            }
                        }
                        KeyCode::ArrowLeft => {
                            info!("Navigate back requested (engine API required)");
                        }
                        KeyCode::ArrowRight => {
                            info!("Navigate forward requested (engine API required)");
                        }
                        KeyCode::KeyL => {
                            info!("Focus address bar requested (engine API required)");
                        }
                        KeyCode::KeyT => {
                            info!("New tab requested (engine API required)");
                        }
                        KeyCode::KeyW => {
                            info!("Close tab requested (engine API required)");
                        }
                        _ => {}
                    }
                }

                Event::AboutToWait => {
                    window_for_loop.request_redraw();
                }

                Event::WindowEvent {
                    event: WindowEvent::RedrawRequested,
                    ..
                } => {
                    // Frame timing
                    let now = Instant::now();
                    let frame_delta = now.duration_since(last_frame_time);
                    last_frame_time = now;
                    frame_count = frame_count.wrapping_add(1);

                    // Tiny render tick; swap for real pipeline later.
                    let render_start = Instant::now();
                    let _ = do_render_tick();
                    // Example when ready:
                    // let _ = rt.block_on(execute_render_frame(&engine_for_loop, render_start));

                    let render_time = render_start.elapsed();
                    if render_time.as_millis() > 16 {
                        info!("Slow frame: {:.2}ms", render_time.as_millis());
                    }
                    if frame_count % 120 == 0 {
                        info!("Frame {}: {:.2}ms", frame_count, render_time.as_millis());
                    }

                    // Update perf stats
                    perf_monitor.record_frame(frame_delta);
                }

                Event::WindowEvent {
                    event: WindowEvent::CursorMoved { position, .. },
                    ..
                } => {
                    info!("Mouse moved to {:?}", position);
                }
                Event::WindowEvent {
                    event: WindowEvent::MouseInput { state, button, .. },
                    ..
                } => {
                    info!("Mouse button {:?} {:?}", button, state);
                }
                Event::WindowEvent {
                    event: WindowEvent::MouseWheel { delta, .. },
                    ..
                } => {
                    info!("Mouse wheel: {:?}", delta);
                }
                Event::WindowEvent {
                    event: WindowEvent::Ime(ime_input),
                    ..
                } => {
                    info!("IME input: {:?}", ime_input);
                }
                Event::WindowEvent {
                    event: WindowEvent::Focused(focused),
                    ..
                } => {
                    info!("Window focus changed: {}", focused);
                }
                _ => {}
            }
        })
        .unwrap();

    #[allow(unreachable_code)]
    Ok(())
}

fn main() -> vulkan_browser_engine::Result<()> {
    let app_config = AppConfig::from_args();

    setup_logging(app_config.log_level, app_config.enable_tracy);
    info!("Starting Vulkan Browser Engine");

    // ONE current-thread runtime for the whole app.
    let rt = Builder::new_current_thread()
        .enable_time()
        .enable_io()
        .build()
        .expect("failed to build single-thread runtime");

    let startup_start = app_config
        .profile_startup
        .then_some(Instant::now());

    let browser_config = BrowserConfig::default();

    if app_config.headless && app_config.benchmark {
        let engine = rt.block_on(BrowserEngine::new(browser_config))?;
        setup_signal_handlers(&rt);
        rt.block_on(run_headless_benchmark(&engine))?;
    } else if app_config.headless {
        let engine = rt.block_on(BrowserEngine::new(browser_config))?;
        setup_signal_handlers(&rt);

        if let Some(url) = &app_config.url {
            rt.block_on(engine.load_url(url))?;
        }

        // Park until Ctrl+C, then shutdown synchronously.
        rt.block_on(signal::ctrl_c()).expect("Failed to listen for ctrl-c");
        info!("Shutting down browser engine...");
        if let Err(e) = rt.block_on(async { engine.shutdown().await }) {
            error!("Shutdown error: {}", e);
        }
    } else {
        run_windowed(app_config, &rt)?;
    }

    if let Some(start) = startup_start {
        info!("Startup completed in {:?}", start.elapsed());
    }

    Ok(())
}
