use std::sync::Arc;
use std::hash::Hasher;
use parking_lot::{RwLock, Mutex};
use dashmap::DashMap;
use thiserror::Error;
use serde_json::Value;
use tokio::sync::Semaphore;
use std::time::{Duration, Instant};
use ahash::AHasher;

pub mod gc;
pub mod jit;
pub mod modules;
pub mod v8_binding;

use gc::{GarbageCollector, Heap as HeapManager};
use jit::{JITCompiler, OptimizationLevel, JSFunction, CompiledFunction};
use modules::ModuleResolver;
use v8_binding::V8Runtime;
use crate::core::dom::Document;
use crate::BrowserConfig;

const MAX_EXECUTION_CONTEXTS: usize = 1000;
const SCRIPT_CACHE_MAX_SIZE: usize = 10000;
const JIT_THRESHOLD_EXECUTIONS: u32 = 3;
const GC_TRIGGER_HEAP_RATIO: f64 = 0.8;

#[derive(Debug, Clone)]
pub struct ModuleCache {
    cache: Arc<DashMap<String, (Value, Instant)>>,
    ttl: Duration,
}

impl ModuleCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(DashMap::new()),
            ttl: Duration::from_secs(3600),
        }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        if let Some(entry) = self.cache.get(key) {
            let (value, timestamp) = entry.value();
            if timestamp.elapsed() < self.ttl {
                return Some(value.clone());
            }
            drop(entry);
            self.cache.remove(key);
        }
        None
    }

    pub fn insert(&self, key: &str, value: &Value) {
        if self.cache.len() >= SCRIPT_CACHE_MAX_SIZE {
            self.evict_expired();
        }
        self.cache.insert(key.to_string(), (value.clone(), Instant::now()));
    }

    fn evict_expired(&self) {
        let now = Instant::now();
        self.cache.retain(|_, (_, timestamp)| now.duration_since(*timestamp) < self.ttl);
    }
}

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
    #[error("Context limit exceeded")]
    ContextLimit,
    #[error("Runtime disposed")]
    Disposed,
}

pub type Result<T> = std::result::Result<T, JSError>;

#[derive(Debug)]
pub struct ExecutionContext {
    pub context_id: u64,
    pub security_token: Option<String>,
    pub module_cache: Arc<ModuleCache>,
    pub created_at: Instant,
    pub last_used: Instant,
}

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
    pub context_count: u32,
    pub cache_hit_rate: f64,
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
            context_count: 0,
            cache_hit_rate: 0.0,
        }
    }
}

#[derive(Debug)]
pub struct ScriptInfo {
    pub source_hash: u64,
    pub filename: String,
    pub is_module: bool,
    pub compilation_time: Duration,
    pub execution_count: u32,
    pub last_execution: Instant,
    pub jit_compiled: bool,
    pub jit_function: Option<Arc<CompiledFunction>>,
}

struct RuntimeCore {
    v8_runtime: V8Runtime,
    heap_stats: HeapStats,
}

#[derive(Debug, Clone)]
struct HeapStats {
    total_bytes: u64,
    used_bytes: u64,
    last_updated: Instant,
}

impl HeapStats {
    fn new() -> Self {
        Self {
            total_bytes: 64 * 1024 * 1024,
            used_bytes: 0,
            last_updated: Instant::now(),
        }
    }

    fn update_usage(&mut self, used: u64) {
        self.used_bytes = used;
        self.last_updated = Instant::now();
    }

    fn usage_ratio(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            self.used_bytes as f64 / self.total_bytes as f64
        }
    }
}

unsafe impl Send for RuntimeCore {}
unsafe impl Sync for RuntimeCore {}

pub struct JSRuntime {
    core: Arc<Mutex<RuntimeCore>>,
    jit_compiler: Arc<JITCompiler>,
    garbage_collector: Arc<Mutex<GarbageCollector>>,
    heap_manager: Arc<HeapManager>,
    module_resolver: Arc<ModuleResolver>,
    execution_contexts: Arc<DashMap<u64, ExecutionContext>>,
    script_cache: Arc<DashMap<u64, ScriptInfo>>,
    performance_metrics: Arc<RwLock<JSPerformanceMetrics>>,
    config: BrowserConfig,
    next_context_id: Arc<Mutex<u64>>,
    context_semaphore: Arc<Semaphore>,
    disposed: Arc<parking_lot::RwLock<bool>>,
    cache_hits: Arc<Mutex<u64>>,
    cache_misses: Arc<Mutex<u64>>,
}

impl JSRuntime {
    pub async fn new(config: &BrowserConfig) -> Result<Self> {
        let v8_platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(v8_platform);
        v8::V8::initialize();

        let v8_runtime = V8Runtime::new()
            .map_err(|e| JSError::RuntimeInit(format!("V8Runtime creation failed: {}", e)))?;

        let heap_stats = HeapStats::new();

        let core = Arc::new(Mutex::new(RuntimeCore {
            v8_runtime,
            heap_stats,
        }));

        let optimization_level = if config.enable_jit {
            OptimizationLevel::Aggressive
        } else {
            OptimizationLevel::None
        };

        let jit_compiler = Arc::new(
            JITCompiler::new(optimization_level)
                .await
                .map_err(|e| JSError::JIT(format!("JIT compiler initialization failed: {}", e)))?
        );

        let heap_manager = Arc::new(HeapManager::new());
        let garbage_collector = Arc::new(Mutex::new(GarbageCollector::new()));
        let module_resolver = Arc::new(ModuleResolver::new());

        let runtime = Self {
            core,
            jit_compiler,
            garbage_collector,
            heap_manager,
            module_resolver,
            execution_contexts: Arc::new(DashMap::new()),
            script_cache: Arc::new(DashMap::new()),
            performance_metrics: Arc::new(RwLock::new(JSPerformanceMetrics::default())),
            config: config.clone(),
            next_context_id: Arc::new(Mutex::new(1)),
            context_semaphore: Arc::new(Semaphore::new(MAX_EXECUTION_CONTEXTS)),
            disposed: Arc::new(parking_lot::RwLock::new(false)),
            cache_hits: Arc::new(Mutex::new(0)),
            cache_misses: Arc::new(Mutex::new(0)),
        };

        runtime.setup_global_apis().await?;
        Ok(runtime)
    }

    async fn setup_global_apis(&self) -> Result<()> {
        Ok(())
    }

    pub async fn create_context(&self) -> Result<u64> {
        if *self.disposed.read() {
            return Err(JSError::Disposed);
        }

        let _permit = self.context_semaphore.acquire().await
            .map_err(|_| JSError::ContextLimit)?;

        let context_id = {
            let mut next_id = self.next_context_id.lock();
            let id = *next_id;
            *next_id += 1;
            id
        };

        let execution_context = ExecutionContext {
            context_id,
            security_token: None,
            module_cache: Arc::new(ModuleCache::new()),
            created_at: Instant::now(),
            last_used: Instant::now(),
        };

        self.execution_contexts.insert(context_id, execution_context);

        {
            let mut metrics = self.performance_metrics.write();
            metrics.context_count = self.execution_contexts.len() as u32;
        }

        Ok(context_id)
    }

    pub async fn execute(&self, script: &str) -> Result<Value> {
        let context_id = self.create_context().await?;
        self.execute_in_context(context_id, script, "inline").await
    }

    pub async fn execute_in_context(&self, context_id: u64, script: &str, filename: &str) -> Result<Value> {
        if *self.disposed.read() {
            return Err(JSError::Disposed);
        }

        let start_time = Instant::now();
        let script_hash = self.calculate_script_hash(script, filename);

        let should_jit = self.update_script_cache_and_check_jit(script_hash, filename).await;

        if should_jit {
            self.trigger_jit_compilation(script_hash, script, filename).await;
        }

        let result = {
            let mut core = self.core.lock();
            
            if let Some(jit_function) = self.get_jit_compiled_function(script_hash).await {
                self.execute_jit_function(&jit_function, context_id).await
            } else {
                core.v8_runtime.execute(script)
                    .map_err(|e| JSError::Execution(e.to_string()))
            }
        }?;

        self.update_context_usage(context_id).await;
        self.update_performance_metrics(start_time).await;
        self.maybe_trigger_gc().await;

        Ok(result)
    }

    fn calculate_script_hash(&self, script: &str, filename: &str) -> u64 {
        let mut hasher = AHasher::default();
        hasher.write(filename.as_bytes());
        hasher.write(script.as_bytes());
        hasher.finish()
    }

    async fn update_script_cache_and_check_jit(&self, script_hash: u64, filename: &str) -> bool {
        if let Some(mut script_info) = self.script_cache.get_mut(&script_hash) {
            script_info.execution_count += 1;
            script_info.last_execution = Instant::now();
            *self.cache_hits.lock() += 1;
            
            return script_info.execution_count > JIT_THRESHOLD_EXECUTIONS 
                && !script_info.jit_compiled 
                && self.config.enable_jit;
        }

        let compilation_start = Instant::now();
        let script_info = ScriptInfo {
            source_hash: script_hash,
            filename: filename.to_string(),
            is_module: false,
            compilation_time: compilation_start.elapsed(),
            execution_count: 1,
            last_execution: Instant::now(),
            jit_compiled: false,
            jit_function: None,
        };

        self.script_cache.insert(script_hash, script_info);
        *self.cache_misses.lock() += 1;

        {
            let mut metrics = self.performance_metrics.write();
            metrics.script_count += 1;
            metrics.compilation_time_us += compilation_start.elapsed().as_micros() as u64;
        }

        false
    }

    async fn trigger_jit_compilation(&self, script_hash: u64, script: &str, filename: &str) {
        let jit_start = Instant::now();
        
        let js_function = JSFunction {
            name: filename.to_string(),
            source_code: script.to_string(),
            body: script.to_string(),
            is_hot: true,
            call_count: 1,
            type_feedback: Default::default(),
            parameters: Vec::new(),
        };

        match self.jit_compiler.compile_function(&js_function).await {
            Ok(compiled_function) => {
                if let Some(mut script_info) = self.script_cache.get_mut(&script_hash) {
                    script_info.jit_compiled = true;
                    script_info.jit_function = Some(Arc::new(compiled_function));
                }

                let mut metrics = self.performance_metrics.write();
                metrics.jit_compilation_time_us += jit_start.elapsed().as_micros() as u64;
            },
            Err(e) => {
                tracing::warn!("JIT compilation failed for {}: {}", filename, e);
            }
        }
    }

    async fn get_jit_compiled_function(&self, script_hash: u64) -> Option<Arc<CompiledFunction>> {
        self.script_cache.get(&script_hash)?.jit_function.clone()
    }

    async fn execute_jit_function(&self, _compiled_function: &CompiledFunction, _context_id: u64) -> Result<Value> {
        Ok(serde_json::Value::Null)
    }

    async fn update_context_usage(&self, context_id: u64) {
        if let Some(mut context) = self.execution_contexts.get_mut(&context_id) {
            context.last_used = Instant::now();
        }
    }

    async fn update_performance_metrics(&self, start_time: Instant) {
        let execution_time = start_time.elapsed();
        let mut metrics = self.performance_metrics.write();
        
        metrics.execution_time_us += execution_time.as_micros() as u64;
        
        let hits = *self.cache_hits.lock();
        let misses = *self.cache_misses.lock();
        let total = hits + misses;
        
        if total > 0 {
            metrics.cache_hit_rate = hits as f64 / total as f64;
        }
    }

    async fn maybe_trigger_gc(&self) {
        let should_gc = {
            let mut core = self.core.lock();
            let current_usage = core.heap_stats.used_bytes + 1024 * 1024;
            core.heap_stats.update_usage(current_usage);
            core.heap_stats.usage_ratio() > GC_TRIGGER_HEAP_RATIO
        };
        
        if should_gc {
            let gc_start = Instant::now();
            self.garbage_collector.lock().collect();
            
            let mut metrics = self.performance_metrics.write();
            metrics.gc_time_us += gc_start.elapsed().as_micros() as u64;
            
            let core = self.core.lock();
            metrics.heap_size_bytes = core.heap_stats.total_bytes;
            metrics.heap_used_bytes = core.heap_stats.used_bytes;
        }
    }

    pub async fn load_module(&self, context_id: u64, module_path: &str) -> Result<Value> {
        if *self.disposed.read() {
            return Err(JSError::Disposed);
        }

        let context = self.execution_contexts.get(&context_id)
            .ok_or_else(|| JSError::Module("Context not found".to_string()))?;

        if let Some(cached_module) = context.module_cache.get(module_path) {
            return Ok(cached_module);
        }

        let module_source = self.module_resolver.resolve(module_path, None)
            .map_err(|e| JSError::Module(format!("Failed to resolve module {}: {}", module_path, e)))?;

        let result = {
            let mut core = self.core.lock();
            core.v8_runtime.execute(&module_source)
                .map_err(|e| JSError::Module(e.to_string()))?
        };

        context.module_cache.insert(module_path, &result);

        {
            let mut metrics = self.performance_metrics.write();
            metrics.module_count += 1;
        }

        Ok(result)
    }

    pub async fn inject_document_api(&self, _document: &Document) -> Result<()> {
        Ok(())
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

    pub async fn inject_serial_api(&self) -> Result<()> {
        Ok(())
    }

    pub async fn inject_usb_api(&self) -> Result<()> {
        Ok(())
    }

    pub async fn inject_bluetooth_api(&self) -> Result<()> {
        Ok(())
    }

    pub async fn inject_gamepad_api(&self) -> Result<()> {
        Ok(())
    }

    pub async fn inject_webrtc_api(&self) -> Result<()> {
        Ok(())
    }

    pub async fn inject_websocket_api(&self) -> Result<()> {
        Ok(())
    }

    pub async fn get_metrics(&self) -> JSPerformanceMetrics {
        *self.performance_metrics.read()
    }

    pub async fn optimize_hot_functions(&self) -> Result<()> {
        if !self.config.enable_jit {
            return Ok(());
        }

        let hot_scripts: Vec<_> = self.script_cache
            .iter()
            .filter(|entry| entry.execution_count > 10 && !entry.jit_compiled)
            .map(|entry| (entry.source_hash, entry.filename.clone()))
            .collect();

        for (script_hash, filename) in hot_scripts {
            if let Some(mut script_info) = self.script_cache.get_mut(&script_hash) {
                let js_function = JSFunction {
                    name: filename.clone(),
                    source_code: "".to_string(),
                    body: "".to_string(),
                    is_hot: true,
                    call_count: script_info.execution_count as u64,
                    type_feedback: Default::default(),
                    parameters: Vec::new(),
                };

                if let Ok(compiled_function) = self.jit_compiler.compile_function(&js_function).await {
                    script_info.jit_compiled = true;
                    script_info.jit_function = Some(Arc::new(compiled_function));
                }
            }
        }

        Ok(())
    }

    pub async fn clear_context(&self, context_id: u64) -> Result<()> {
        self.execution_contexts.remove(&context_id);
        
        {
            let mut metrics = self.performance_metrics.write();
            metrics.context_count = self.execution_contexts.len() as u32;
        }

        self.maybe_trigger_gc().await;
        Ok(())
    }

    pub async fn cleanup_expired_contexts(&self) {
        let now = Instant::now();
        let ttl = Duration::from_secs(1800);

        self.execution_contexts.retain(|_, context| {
            now.duration_since(context.last_used) < ttl
        });

        {
            let mut metrics = self.performance_metrics.write();
            metrics.context_count = self.execution_contexts.len() as u32;
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        {
            let mut disposed = self.disposed.write();
            if *disposed {
                return Ok(());
            }
            *disposed = true;
        }

        self.execution_contexts.clear();
        self.script_cache.clear();

        unsafe {
            v8::V8::dispose();
        }

        Ok(())
    }
}

impl Drop for JSRuntime {
    fn drop(&mut self) {
        if !*self.disposed.read() {
            let _ = futures::executor::block_on(self.shutdown());
        }
    }
}