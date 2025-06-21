use std::env;
use std::sync::Arc;
use std::time::Instant;
use tokio::signal;
use tracing::{info, error, Level};
use tracing_subscriber::FmtSubscriber;
use vulkan_browser_engine::{BrowserEngine, BrowserConfig, platform::Window};
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
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
        
        let mut i = 1;
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

async fn run_headless_benchmark(engine: Arc<BrowserEngine>) -> vulkan_browser_engine::Result<()> {
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
    
    let total_time = start.elapsed();
    println!("Total benchmark time: {:?}", total_time);
    
    Ok(())
}

async fn run_windowed(app_config: AppConfig) -> vulkan_browser_engine::Result<()> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Vulkan Browser Engine")
        .with_inner_size(winit::dpi::LogicalSize::new(1920, 1080))
        .build(&event_loop)
        .expect("Failed to create window");
    
    let window = Arc::new(window);
    
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
    
    let engine = Arc::new(BrowserEngine::new(browser_config).await?);
    
    if let Some(url) = app_config.url {
        engine.load_url(&url).await?;
    } else {
        engine.load_url("data:text/html,<html><body><h1>Vulkan Browser Engine</h1><p>Ready for navigation</p></body></html>").await?;
    }
    
    let engine_clone = engine.clone();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;
        
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                *control_flow = ControlFlow::Exit;
            }
            Event::WindowEvent { event: WindowEvent::Resized(size), .. } => {
                let engine = engine_clone.clone();
                tokio::spawn(async move {
                    if let Err(e) = engine.resize_viewport(size.width, size.height).await {
                        error!("Failed to resize viewport: {}", e);
                    }
                });
            }
            Event::WindowEvent { event: WindowEvent::KeyboardInput { input, .. }, .. } => {
                if input.virtual_keycode == Some(winit::event::VirtualKeyCode::F5) {
                    let engine = engine_clone.clone();
                    tokio::spawn(async move {
                        if let Err(e) = engine.reload().await {
                            error!("Failed to reload: {}", e);
                        }
                    });
                }
            }
            Event::RedrawRequested(_) => {
            }
            Event::MainEventsCleared => {
                window.request_redraw();
            }
            _ => {}
        }
    });
}

fn setup_logging(level: Level, enable_tracy: bool) {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .finish();
    
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set tracing subscriber");
    
    if enable_tracy {
        #[cfg(feature = "tracy")]
        tracy_client::Client::start();
    }
}

async fn setup_signal_handlers(engine: Arc<BrowserEngine>) {
    tokio::spawn(async move {
        if let Ok(()) = signal::ctrl_c().await {
            info!("Received SIGINT, shutting down gracefully");
            if let Err(e) = engine.shutdown().await {
                error!("Error during shutdown: {}", e);
            }
            std::process::exit(0);
        }
    });
}

#[tokio::main]
async fn main() -> vulkan_browser_engine::Result<()> {
    let app_config = AppConfig::from_args();
    
    setup_logging(app_config.log_level, app_config.enable_tracy);
    
    info!("Starting Vulkan Browser Engine");
    
    let startup_start = if app_config.profile_startup {
        Some(Instant::now())
    } else {
        None
    };
    
    let browser_config = BrowserConfig::default();
    
    if app_config.headless && app_config.benchmark {
        let engine = Arc::new(BrowserEngine::new(browser_config).await?);
        setup_signal_handlers(engine.clone()).await;
        run_headless_benchmark(engine).await?;
    } else if app_config.headless {
        let engine = Arc::new(BrowserEngine::new(browser_config).await?);
        setup_signal_handlers(engine.clone()).await;
        
        if let Some(url) = app_config.url {
            engine.load_url(&url).await?;
        }
        
        tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl-c");
        engine.shutdown().await?;
    } else {
        run_windowed(app_config).await?;
    }
    
    if let Some(start) = startup_start {
        info!("Startup completed in {:?}", start.elapsed());
    }
    
    Ok(())
}