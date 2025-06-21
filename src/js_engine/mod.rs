use std::sync::Arc;
use std::collections::HashMap;
use std::ffi::CString;
use parking_lot::{RwLock, Mutex};
use dashmap::DashMap;
use thiserror::Error;
use serde_json::Value;
use tokio::sync::mpsc;

pub mod gc;
pub mod jit;
pub mod modules;
pub mod v8_binding;

use gc::{GarbageCollector, HeapManager};
use jit::{JITCompiler, OptimizationLevel};
use modules::{ModuleResolver, ModuleCache};
use v8_binding::{V8Runtime, V8Callbacks};
use crate::core::dom::Document;
use crate::BrowserConfig;

#[derive(Error, Debug)]
pub enum JSError {
    #[error("Runtime initialization failed: {0}")]
    RuntimeInit(String),
    #[error("Script compilation failed: {0}")]
    Compilation(String),
    #[error("Script execution failed: {0}")]
    Execution(String),
    #[error("Memory allocation failed: {0}")]
    Memory(String),
    #[error("Module loading failed: {0}")]
    Module(String),
    #[error("Type conversion failed: {0}")]
    TypeConversion(String),
    #[error("Security violation: {0}")]
    Security(String),
    #[error("JIT compilation failed: {0}")]
    JIT(String),
}

pub type Result<T> = std::result::Result<T, JSError>;

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub global_object: v8::Global<v8::Object>,
    pub isolate_ptr: *mut v8::Isolate,
    pub context_id: u64,
    pub security_token: Option<String>,
    pub module_cache: Arc<ModuleCache>,
}

unsafe impl Send for ExecutionContext {}
unsafe impl Sync for ExecutionContext {}

#[derive(Debug, Clone, Copy)]
pub struct JSPerformanceMetrics {
    pub compilation_time_us: u64,
    pub execution_time_us: u64,
    pub gc_time_us: u64,
    pub heap_size_bytes: u64,
    pub heap_used_bytes: u64,
    pub jit_compilation_time_us: u64,
    pub script_count: u32,
    pub module_count: u32,
}

impl Default for JSPerformanceMetrics {
    fn default() -> Self {
        Self {
            compilation_time_us: 0,
            execution_time_us: 0,
            gc_time_us: 0,
            heap_size_bytes: 0,
            heap_used_bytes: 0,
            jit_compilation_time_us: 0,
            script_count: 0,
            module_count: 0,
        }
    }
}

pub struct ScriptInfo {
    pub script: v8::Global<v8::Script>,
    pub source_code: String,
    pub filename: String,
    pub is_module: bool,
    pub compilation_time: std::time::Instant,
    pub execution_count: u32,
    pub last_execution: std::time::Instant,
    pub jit_compiled: bool,
}

pub struct JSRuntime {
    v8_runtime: Arc<RwLock<V8Runtime>>,
    jit_compiler: Arc<JITCompiler>,
    garbage_collector: Arc<GarbageCollector>,
    heap_manager: Arc<HeapManager>,
    module_resolver: Arc<ModuleResolver>,
    execution_contexts: Arc<DashMap<u64, ExecutionContext>>,
    script_cache: Arc<DashMap<String, ScriptInfo>>,
    global_functions: Arc<DashMap<String, v8::Global<v8::Function>>>,
    performance_metrics: Arc<RwLock<JSPerformanceMetrics>>,
    config: BrowserConfig,
    next_context_id: Arc<Mutex<u64>>,
    message_queue: Arc<Mutex<mpsc::UnboundedReceiver<JSMessage>>>,
    message_sender: mpsc::UnboundedSender<JSMessage>,
    chrome_apis_enabled: bool,
}

#[derive(Debug, Clone)]
pub enum JSMessage {
    ExecuteScript {
        context_id: u64,
        script: String,
        filename: String,
        response_tx: tokio::sync::oneshot::Sender<Result<Value>>,
    },
    LoadModule {
        context_id: u64,
        module_path: String,
        response_tx: tokio::sync::oneshot::Sender<Result<Value>>,
    },
    GarbageCollect {
        context_id: Option<u64>,
        force: bool,
    },
    GetMetrics {
        response_tx: tokio::sync::oneshot::Sender<JSPerformanceMetrics>,
    },
}

impl JSRuntime {
    pub async fn new(config: &BrowserConfig) -> Result<Self> {
        let v8_platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(v8_platform);
        v8::V8::initialize();

        let v8_runtime = Arc::new(RwLock::new(V8Runtime::new(config).await?));
        
        let jit_compiler = Arc::new(JITCompiler::new(
            if config.enable_jit { OptimizationLevel::Aggressive } else { OptimizationLevel::None }
        ).await?);

        let heap_manager = Arc::new(HeapManager::new(config.max_memory_mb * 1024 * 1024)?);
        let garbage_collector = Arc::new(GarbageCollector::new(heap_manager.clone()).await?);
        let module_resolver = Arc::new(ModuleResolver::new().await?);

        let (message_tx, message_rx) = mpsc::unbounded_channel();

        let runtime = Self {
            v8_runtime,
            jit_compiler,
            garbage_collector,
            heap_manager,
            module_resolver,
            execution_contexts: Arc::new(DashMap::new()),
            script_cache: Arc::new(DashMap::new()),
            global_functions: Arc::new(DashMap::new()),
            performance_metrics: Arc::new(RwLock::new(JSPerformanceMetrics::default())),
            config: config.clone(),
            next_context_id: Arc::new(Mutex::new(1)),
            message_queue: Arc::new(Mutex::new(message_rx)),
            message_sender: message_tx,
            chrome_apis_enabled: config.enable_chrome_apis,
        };

        runtime.setup_global_apis().await?;
        runtime.start_message_loop().await;

        Ok(runtime)
    }

    async fn setup_global_apis(&self) -> Result<()> {
        let mut v8_runtime = self.v8_runtime.write();
        
        v8_runtime.setup_console_api().await?;
        v8_runtime.setup_timer_api().await?;
        v8_runtime.setup_fetch_api().await?;
        v8_runtime.setup_storage_api().await?;
        v8_runtime.setup_crypto_api().await?;
        
        if self.chrome_apis_enabled {
            v8_runtime.setup_chrome_apis().await?;
        }

        Ok(())
    }

    async fn start_message_loop(&self) {
        let message_queue = self.message_queue.clone();
        let runtime = Arc::new(self.clone());

        tokio::spawn(async move {
            let mut receiver = message_queue.lock();
            while let Some(message) = receiver.recv().await {
                runtime.handle_message(message).await;
            }
        });
    }

    async fn handle_message(&self, message: JSMessage) {
        match message {
            JSMessage::ExecuteScript { context_id, script, filename, response_tx } => {
                let result = self.execute_script_internal(context_id, &script, &filename).await;
                let _ = response_tx.send(result);
            },
            JSMessage::LoadModule { context_id, module_path, response_tx } => {
                let result = self.load_module_internal(context_id, &module_path).await;
                let _ = response_tx.send(result);
            },
            JSMessage::GarbageCollect { context_id, force } => {
                if let Some(ctx_id) = context_id {
                    self.collect_garbage_for_context(ctx_id, force).await;
                } else {
                    self.collect_garbage_all(force).await;
                }
            },
            JSMessage::GetMetrics { response_tx } => {
                let metrics = *self.performance_metrics.read();
                let _ = response_tx.send(metrics);
            },
        }
    }

    pub async fn create_context(&self) -> Result<u64> {
        let context_id = {
            let mut next_id = self.next_context_id.lock();
            let id = *next_id;
            *next_id += 1;
            id
        };

        let mut v8_runtime = self.v8_runtime.write();
        let (context, global_object) = v8_runtime.create_context().await?;
        
        let execution_context = ExecutionContext {
            global_object,
            isolate_ptr: v8_runtime.isolate_ptr(),
            context_id,
            security_token: None,
            module_cache: Arc::new(ModuleCache::new()),
        };

        self.execution_contexts.insert(context_id, execution_context);

        Ok(context_id)
    }

    pub async fn execute(&self, script: &str) -> Result<Value> {
        let context_id = self.create_context().await?;
        self.execute_in_context(context_id, script, "inline").await
    }

    pub async fn execute_in_context(&self, context_id: u64, script: &str, filename: &str) -> Result<Value> {
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        
        let message = JSMessage::ExecuteScript {
            context_id,
            script: script.to_string(),
            filename: filename.to_string(),
            response_tx,
        };

        self.message_sender.send(message)
            .map_err(|e| JSError::Execution(format!("Failed to send message: {}", e)))?;

        response_rx.await
            .map_err(|e| JSError::Execution(format!("Failed to receive response: {}", e)))?
    }

    async fn execute_script_internal(&self, context_id: u64, script: &str, filename: &str) -> Result<Value> {
        let start_time = std::time::Instant::now();

        let context = self.execution_contexts.get(&context_id)
            .ok_or_else(|| JSError::Execution("Context not found".to_string()))?;

        let script_hash = format!("{}:{}", filename, ahash::AHasher::default().write(script.as_bytes()));
        
        let compiled_script = if let Some(cached_script) = self.script_cache.get(&script_hash) {
            cached_script.value().script.clone()
        } else {
            let compilation_start = std::time::Instant::now();
            
            let mut v8_runtime = self.v8_runtime.write();
            let compiled = v8_runtime.compile_script(script, filename).await?;
            
            let compilation_time = compilation_start.elapsed();
            
            let script_info = ScriptInfo {
                script: compiled.clone(),
                source_code: script.to_string(),
                filename: filename.to_string(),
                is_module: false,
                compilation_time: std::time::Instant::now(),
                execution_count: 0,
                last_execution: std::time::Instant::now(),
                jit_compiled: false,
            };

            self.script_cache.insert(script_hash.clone(), script_info);
            
            {
                let mut metrics = self.performance_metrics.write();
                metrics.compilation_time_us += compilation_time.as_micros() as u64;
                metrics.script_count += 1;
            }

            compiled
        };

        let should_jit = {
            if let Some(mut script_info) = self.script_cache.get_mut(&script_hash) {
                script_info.execution_count += 1;
                script_info.last_execution = std::time::Instant::now();
                
                script_info.execution_count > 5 && !script_info.jit_compiled && self.config.enable_jit
            } else {
                false
            }
        };

        if should_jit {
            let jit_start = std::time::Instant::now();
            
            if let Err(e) = self.jit_compiler.compile_hot_function(script, filename).await {
                tracing::warn!("JIT compilation failed: {}", e);
            } else {
                if let Some(mut script_info) = self.script_cache.get_mut(&script_hash) {
                    script_info.jit_compiled = true;
                }
                
                let jit_time = jit_start.elapsed();
                let mut metrics = self.performance_metrics.write();
                metrics.jit_compilation_time_us += jit_time.as_micros() as u64;
            }
        }

        let mut v8_runtime = self.v8_runtime.write();
        let result = v8_runtime.execute_script(&compiled_script, context_id).await?;

        let execution_time = start_time.elapsed();
        
        {
            let mut metrics = self.performance_metrics.write();
            metrics.execution_time_us += execution_time.as_micros() as u64;
            
            let heap_stats = v8_runtime.get_heap_statistics();
            metrics.heap_size_bytes = heap_stats.total_heap_size();
            metrics.heap_used_bytes = heap_stats.used_heap_size();
        }

        if self.heap_manager.should_trigger_gc().await {
            self.trigger_incremental_gc().await;
        }

        Ok(result)
    }

    async fn load_module_internal(&self, context_id: u64, module_path: &str) -> Result<Value> {
        let context = self.execution_contexts.get(&context_id)
            .ok_or_else(|| JSError::Module("Context not found".to_string()))?;

        if let Some(cached_module) = context.module_cache.get(module_path).await {
            return Ok(cached_module);
        }

        let module_source = self.module_resolver.resolve(module_path).await
            .map_err(|e| JSError::Module(format!("Failed to resolve module {}: {}", module_path, e)))?;

        let mut v8_runtime = self.v8_runtime.write();
        let module = v8_runtime.compile_module(&module_source, module_path).await?;
        let result = v8_runtime.execute_module(&module, context_id).await?;

        context.module_cache.insert(module_path, &result).await;

        {
            let mut metrics = self.performance_metrics.write();
            metrics.module_count += 1;
        }

        Ok(result)
    }

    pub async fn inject_document_api(&self, document: &Document) -> Result<()> {
        let mut v8_runtime = self.v8_runtime.write();
        v8_runtime.inject_document_api(document).await
    }

    pub async fn execute_inline_scripts(&self, document: &Document) -> Result<()> {
        let scripts = document.get_inline_scripts();
        
        for script in scripts {
            if let Err(e) = self.execute(&script.content).await {
                tracing::warn!("Failed to execute inline script: {}", e);
            }
        }

        Ok(())
    }

    pub async fn inject_serial_api(&mut self) -> Result<()> {
        let mut v8_runtime = self.v8_runtime.write();
        v8_runtime.inject_serial_api().await
    }

    pub async fn inject_usb_api(&mut self) -> Result<()> {
        let mut v8_runtime = self.v8_runtime.write();
        v8_runtime.inject_usb_api().await
    }

    pub async fn inject_bluetooth_api(&mut self) -> Result<()> {
        let mut v8_runtime = self.v8_runtime.write();
        v8_runtime.inject_bluetooth_api().await
    }

    pub async fn inject_gamepad_api(&mut self) -> Result<()> {
        let mut v8_runtime = self.v8_runtime.write();
        v8_runtime.inject_gamepad_api().await
    }

    pub async fn inject_webrtc_api(&mut self) -> Result<()> {
        let mut v8_runtime = self.v8_runtime.write();
        v8_runtime.inject_webrtc_api().await
    }

    pub async fn inject_websocket_api(&mut self) -> Result<()> {
        let mut v8_runtime = self.v8_runtime.write();
        v8_runtime.inject_websocket_api().await
    }

    async fn trigger_incremental_gc(&self) {
        let gc_start = std::time::Instant::now();
        
        self.garbage_collector.collect_incremental().await;
        
        let gc_time = gc_start.elapsed();
        let mut metrics = self.performance_metrics.write();
        metrics.gc_time_us += gc_time.as_micros() as u64;
    }

    async fn collect_garbage_for_context(&self, context_id: u64, force: bool) {
        if force {
            self.garbage_collector.collect_full().await;
        } else {
            self.garbage_collector.collect_incremental().await;
        }
    }

    async fn collect_garbage_all(&self, force: bool) {
        if force {
            self.garbage_collector.collect_full().await;
        } else {
            self.garbage_collector.collect_incremental().await;
        }
    }

    pub async fn get_metrics(&self) -> JSPerformanceMetrics {
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        
        let message = JSMessage::GetMetrics { response_tx };
        
        if let Ok(()) = self.message_sender.send(message) {
            if let Ok(metrics) = response_rx.await {
                return metrics;
            }
        }

        *self.performance_metrics.read()
    }

    pub async fn optimize_hot_functions(&self) -> Result<()> {
        if !self.config.enable_jit {
            return Ok(());
        }

        let hot_scripts: Vec<_> = self.script_cache
            .iter()
            .filter(|entry| entry.execution_count > 10 && !entry.jit_compiled)
            .map(|entry| (entry.key().clone(), entry.source_code.clone(), entry.filename.clone()))
            .collect();

        for (hash, source, filename) in hot_scripts {
            if let Err(e) = self.jit_compiler.compile_hot_function(&source, &filename).await {
                tracing::warn!("Failed to JIT compile {}: {}", filename, e);
                continue;
            }

            if let Some(mut script_info) = self.script_cache.get_mut(&hash) {
                script_info.jit_compiled = true;
            }
        }

        Ok(())
    }

    pub async fn clear_context(&self, context_id: u64) -> Result<()> {
        self.execution_contexts.remove(&context_id);
        self.collect_garbage_for_context(context_id, false).await;
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.garbage_collector.shutdown().await;
        self.jit_compiler.shutdown().await;
        
        {
            let mut v8_runtime = self.v8_runtime.write();
            v8_runtime.shutdown().await?;
        }

        unsafe {
            v8::V8::dispose();
        }

        Ok(())
    }
}

impl Clone for JSRuntime {
    fn clone(&self) -> Self {
        Self {
            v8_runtime: self.v8_runtime.clone(),
            jit_compiler: self.jit_compiler.clone(),
            garbage_collector: self.garbage_collector.clone(),
            heap_manager: self.heap_manager.clone(),
            module_resolver: self.module_resolver.clone(),
            execution_contexts: self.execution_contexts.clone(),
            script_cache: self.script_cache.clone(),
            global_functions: self.global_functions.clone(),
            performance_metrics: self.performance_metrics.clone(),
            config: self.config.clone(),
            next_context_id: self.next_context_id.clone(),
            message_queue: self.message_queue.clone(),
            message_sender: self.message_sender.clone(),
            chrome_apis_enabled: self.chrome_apis_enabled,
        }
    }
}