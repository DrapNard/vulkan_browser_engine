use dashmap::DashMap;
use parking_lot::Mutex;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

use cranelift::prelude::*;
use cranelift_codegen::ir::Function;
use cranelift_codegen::ir::UserFuncName;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context as CraneliftContext;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use target_lexicon::Triple;

#[derive(Error, Debug)]
pub enum JITError {
    #[error("Compilation failed: {0}")]
    Compilation(String),
    #[error("Code generation failed: {0}")]
    CodeGeneration(String),
    #[error("Optimization failed: {0}")]
    Optimization(String),
    #[error("Function not found: {0}")]
    FunctionNotFound(String),
    #[error("Type error: {0}")]
    TypeError(String),
    #[error("Memory allocation failed: {0}")]
    Memory(String),
}

pub type Result<T> = std::result::Result<T, JITError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptimizationLevel {
    None,
    Basic,
    Aggressive,
    Debug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    I32,
    I64,
    F32,
    F64,
    Pointer,
    Boolean,
    Object,
    String,
    Function,
    Undefined,
}

impl ValueType {
    fn to_cranelift_type(self) -> Type {
        match self {
            ValueType::I32 | ValueType::Boolean => types::I32,
            ValueType::I64 => types::I64,
            ValueType::F32 => types::F32,
            ValueType::F64 => types::F64,
            ValueType::Pointer | ValueType::Object | ValueType::String | ValueType::Function => {
                types::I64
            }
            ValueType::Undefined => types::I64,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JSValue {
    pub value_type: ValueType,
    pub data: u64,
}

impl JSValue {
    pub fn new_undefined() -> Self {
        Self {
            value_type: ValueType::Undefined,
            data: 0,
        }
    }

    pub fn new_number(n: f64) -> Self {
        Self {
            value_type: ValueType::F64,
            data: n.to_bits(),
        }
    }

    pub fn new_boolean(b: bool) -> Self {
        Self {
            value_type: ValueType::Boolean,
            data: if b { 1 } else { 0 },
        }
    }

    pub fn new_object(ptr: *const u8) -> Self {
        Self {
            value_type: ValueType::Object,
            data: ptr as u64,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompiledFunction {
    pub func_id: FuncId,
    pub func_ptr: *const u8,
    pub signature: Signature,
    pub source_hash: u64,
    pub optimization_level: OptimizationLevel,
    pub compilation_time: std::time::Duration,
    pub execution_count: u64,
    pub total_execution_time: std::time::Duration,
}

unsafe impl Send for CompiledFunction {}
unsafe impl Sync for CompiledFunction {}

#[derive(Debug, Clone)]
pub struct JSFunction {
    pub name: String,
    pub source_code: String,
    pub parameters: Vec<String>,
    pub body: String,
    pub is_hot: bool,
    pub call_count: u64,
    pub type_feedback: TypeFeedback,
}

#[derive(Debug, Clone)]
pub struct TypeFeedback {
    pub parameter_types: SmallVec<[ValueType; 8]>,
    pub return_type: ValueType,
    pub observed_types: HashMap<String, SmallVec<[ValueType; 4]>>,
    pub branch_frequencies: HashMap<usize, f32>,
}

impl Default for TypeFeedback {
    fn default() -> Self {
        Self {
            parameter_types: SmallVec::new(),
            return_type: ValueType::Undefined,
            observed_types: HashMap::new(),
            branch_frequencies: HashMap::new(),
        }
    }
}

pub struct ProfilerData {
    pub hot_functions: Arc<DashMap<String, f64>>,
    pub type_feedback: Arc<DashMap<String, TypeFeedback>>,
    pub call_graph: Arc<DashMap<String, Vec<String>>>,
    pub deoptimization_points: Arc<DashMap<String, Vec<usize>>>,
}

impl Default for ProfilerData {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfilerData {
    pub fn new() -> Self {
        Self {
            hot_functions: Arc::new(DashMap::new()),
            type_feedback: Arc::new(DashMap::new()),
            call_graph: Arc::new(DashMap::new()),
            deoptimization_points: Arc::new(DashMap::new()),
        }
    }

    pub fn record_function_call(&self, function_name: &str, execution_time: f64) {
        self.hot_functions
            .entry(function_name.to_string())
            .and_modify(|time| *time += execution_time)
            .or_insert(execution_time);
    }

    pub fn update_type_feedback(&self, function_name: &str, feedback: TypeFeedback) {
        self.type_feedback
            .insert(function_name.to_string(), feedback);
    }

    pub fn is_hot_function(&self, function_name: &str, threshold: f64) -> bool {
        self.hot_functions
            .get(function_name)
            .map(|time| *time.value() > threshold)
            .unwrap_or(false)
    }
}

pub struct JITCompiler {
    module: Arc<Mutex<JITModule>>,
    builder_context: Arc<Mutex<FunctionBuilderContext>>,
    cranelift_context: Arc<Mutex<CraneliftContext>>,
    compiled_functions: Arc<DashMap<String, CompiledFunction>>,
    optimization_level: OptimizationLevel,
    profiler_data: Arc<ProfilerData>,
    runtime_helpers: Arc<DashMap<String, *const u8>>,
    type_specializations: Arc<DashMap<String, HashMap<String, FuncId>>>,
    deoptimization_stubs: Arc<DashMap<String, FuncId>>,
}

impl JITCompiler {
    pub async fn new(optimization_level: OptimizationLevel) -> Result<Self> {
        let mut flag_builder = settings::builder();
        flag_builder.set("use_colocated_libcalls", "false").unwrap();
        flag_builder.set("is_pic", "false").unwrap();
        flag_builder
            .set(
                "enable_verifier",
                cfg!(debug_assertions).to_string().as_str(),
            )
            .unwrap();

        match optimization_level {
            OptimizationLevel::None => {
                flag_builder.set("opt_level", "none").unwrap();
            }
            OptimizationLevel::Basic => {
                flag_builder.set("opt_level", "speed").unwrap();
            }
            OptimizationLevel::Aggressive => {
                flag_builder.set("opt_level", "speed_and_size").unwrap();
            }
            OptimizationLevel::Debug => {
                flag_builder.set("opt_level", "none").unwrap();
                flag_builder.set("enable_verifier", "true").unwrap();
            }
        }

        let isa_flags = settings::Flags::new(flag_builder);
        let isa = cranelift_native::builder()
            .unwrap_or_else(|_| cranelift_codegen::isa::lookup(Triple::host()).unwrap())
            .finish(isa_flags)
            .map_err(|e| JITError::Compilation(e.to_string()))?;

        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let module = JITModule::new(builder);

        let compiler = Self {
            module: Arc::new(Mutex::new(module)),
            builder_context: Arc::new(Mutex::new(FunctionBuilderContext::new())),
            cranelift_context: Arc::new(Mutex::new(CraneliftContext::new())),
            compiled_functions: Arc::new(DashMap::new()),
            optimization_level,
            profiler_data: Arc::new(ProfilerData::new()),
            runtime_helpers: Arc::new(DashMap::new()),
            type_specializations: Arc::new(DashMap::new()),
            deoptimization_stubs: Arc::new(DashMap::new()),
        };

        compiler.setup_runtime_helpers().await?;

        Ok(compiler)
    }

    async fn setup_runtime_helpers(&self) -> Result<()> {
        let runtime_functions = [
            ("js_add", js_add as *const u8),
            ("js_subtract", js_subtract as *const u8),
            ("js_multiply", js_multiply as *const u8),
            ("js_divide", js_divide as *const u8),
            ("js_equals", js_equals as *const u8),
            ("js_typeof", js_typeof as *const u8),
            ("js_to_number", js_to_number as *const u8),
            ("js_to_string", js_to_string as *const u8),
            ("js_property_get", js_property_get as *const u8),
            ("js_property_set", js_property_set as *const u8),
            ("js_function_call", js_function_call as *const u8),
            ("js_new_object", js_new_object as *const u8),
            ("js_gc_barrier", js_gc_barrier as *const u8),
            ("js_deoptimize", js_deoptimize as *const u8),
        ];

        for (name, ptr) in runtime_functions.iter() {
            self.runtime_helpers.insert(name.to_string(), *ptr);
        }

        Ok(())
    }

    pub async fn compile_function(&self, js_function: &JSFunction) -> Result<CompiledFunction> {
        if let Some(compiled) = self.compiled_functions.get(&js_function.name) {
            return Ok(compiled.clone());
        }

        let compilation_start = std::time::Instant::now();

        let (func_id, func_ptr, signature) = {
            let mut module = self.module.lock();
            let mut context = self.cranelift_context.lock();
            let mut builder_context = self.builder_context.lock();

            let signature = self.create_function_signature(&js_function.parameters)?;

            let func_id = module
                .declare_function(&js_function.name, Linkage::Local, &signature)
                .map_err(|e| JITError::Compilation(e.to_string()))?;

            context.func = Function::with_name_signature(
                UserFuncName::user(0, func_id.as_u32()),
                signature.clone(),
            );

            {
                let builder = FunctionBuilder::new(&mut context.func, &mut builder_context);
                self.compile_function_body(builder, js_function)?;
            }

            if let Err(errors) = module.define_function(func_id, &mut context) {
                return Err(JITError::CodeGeneration(format!(
                    "Function definition failed: {:?}",
                    errors
                )));
            }

            module.clear_context(&mut context);
            module.finalize_definitions();

            let func_ptr = module.get_finalized_function(func_id);

            (func_id, func_ptr, signature)
        };

        let compilation_time = compilation_start.elapsed();

        let compiled_function = CompiledFunction {
            func_id,
            func_ptr,
            signature,
            source_hash: self.compute_source_hash(&js_function.source_code),
            optimization_level: self.optimization_level,
            compilation_time,
            execution_count: 0,
            total_execution_time: std::time::Duration::default(),
        };

        self.compiled_functions
            .insert(js_function.name.clone(), compiled_function.clone());

        Ok(compiled_function)
    }

    fn create_function_signature(&self, parameters: &[String]) -> Result<Signature> {
        let call_conv = cranelift_codegen::isa::CallConv::SystemV;
        let mut sig = Signature::new(call_conv);

        for _ in parameters {
            sig.params.push(AbiParam::new(types::I64));
        }

        sig.returns.push(AbiParam::new(types::I64));

        Ok(sig)
    }

    fn compile_function_body(
        &self,
        mut builder: FunctionBuilder,
        _js_function: &JSFunction,
    ) -> Result<()> {
        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);
        let undefined_value = builder.ins().iconst(types::I64, 0);
        builder.ins().return_(&[undefined_value]);
        builder.finalize();
        Ok(())
    }

    pub async fn specialize_function(
        &self,
        function_name: &str,
        types: &[ValueType],
    ) -> Result<FuncId> {
        let specialization_key = format!("{}_{:?}", function_name, types);

        if let Some(specializations) = self.type_specializations.get(function_name) {
            if let Some(&func_id) = specializations.get(&specialization_key) {
                return Ok(func_id);
            }
        }

        let original_function = self
            .compiled_functions
            .get(function_name)
            .ok_or_else(|| JITError::FunctionNotFound(function_name.to_string()))?;

        let mut module = self.module.lock();
        let mut context = self.cranelift_context.lock();
        let mut builder_context = self.builder_context.lock();

        let specialized_name = format!("{}_specialized_{}", function_name, fastrand::u64(..));
        let signature = original_function.signature.clone();

        let func_id = module
            .declare_function(&specialized_name, Linkage::Local, &signature)
            .map_err(|e| JITError::Compilation(e.to_string()))?;

        context.func =
            Function::with_name_signature(UserFuncName::user(0, func_id.as_u32()), signature);

        {
            let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);

            let entry_block = builder.create_block();
            builder.append_block_params_for_function_params(entry_block);
            builder.switch_to_block(entry_block);
            builder.seal_block(entry_block);

            let result = builder.ins().iconst(types::I64, 42);
            builder.ins().return_(&[result]);
            builder.finalize();
        }

        if let Err(errors) = module.define_function(func_id, &mut context) {
            return Err(JITError::CodeGeneration(format!(
                "Specialization failed: {:?}",
                errors
            )));
        }

        module.clear_context(&mut context);

        self.type_specializations
            .entry(function_name.to_string())
            .or_default()
            .insert(specialization_key, func_id);

        Ok(func_id)
    }

    pub fn record_type_feedback(&self, function_name: &str, feedback: TypeFeedback) {
        self.profiler_data
            .update_type_feedback(function_name, feedback);
    }

    pub async fn trigger_recompilation(&self, function_name: &str) -> Result<()> {
        if let Some(feedback) = self.profiler_data.type_feedback.get(function_name) {
            let types: Vec<ValueType> = feedback.parameter_types.iter().cloned().collect();
            self.specialize_function(function_name, &types).await?;
        }

        Ok(())
    }

    pub fn should_deoptimize(&self, function_name: &str) -> bool {
        self.profiler_data
            .deoptimization_points
            .get(function_name)
            .map(|points| !points.is_empty())
            .unwrap_or(false)
    }

    fn compute_source_hash(&self, source: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = ahash::AHasher::default();
        source.hash(&mut hasher);
        hasher.finish()
    }

    pub async fn get_function(&self, name: &str) -> Option<CompiledFunction> {
        self.compiled_functions.get(name).map(|entry| entry.clone())
    }

    pub async fn optimize_all(&self) -> Result<()> {
        let hot_functions: Vec<String> = self
            .profiler_data
            .hot_functions
            .iter()
            .filter(|entry| *entry.value() > 100.0)
            .map(|entry| entry.key().clone())
            .collect();

        for function_name in hot_functions {
            if let Some(compiled) = self.compiled_functions.get(&function_name) {
                if compiled.optimization_level == OptimizationLevel::None {
                    self.trigger_recompilation(&function_name).await?;
                }
            }
        }

        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.compiled_functions.clear();
        self.profiler_data.hot_functions.clear();
        self.profiler_data.type_feedback.clear();
        Ok(())
    }
}

extern "C" fn js_add(_a: u64, _b: u64) -> u64 {
    0
}
extern "C" fn js_subtract(_a: u64, _b: u64) -> u64 {
    0
}
extern "C" fn js_multiply(_a: u64, _b: u64) -> u64 {
    0
}
extern "C" fn js_divide(_a: u64, _b: u64) -> u64 {
    0
}
extern "C" fn js_equals(_a: u64, _b: u64) -> u32 {
    0
}
extern "C" fn js_typeof(_value: u64) -> u64 {
    0
}
extern "C" fn js_to_number(_value: u64) -> f64 {
    0.0
}
extern "C" fn js_to_string(_value: u64) -> u64 {
    0
}
extern "C" fn js_property_get(_object: u64, _property: u64) -> u64 {
    0
}
extern "C" fn js_property_set(_object: u64, _property: u64, _value: u64) -> u64 {
    0
}
extern "C" fn js_function_call(_function: u64, _this: u64, _argc: u32) -> u64 {
    0
}
extern "C" fn js_new_object() -> u64 {
    0
}
extern "C" fn js_gc_barrier(_value: u64) -> u64 {
    0
}
extern "C" fn js_deoptimize(_reason: u64) -> u64 {
    0
}
