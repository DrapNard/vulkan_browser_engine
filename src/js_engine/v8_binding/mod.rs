pub mod callbacks;

pub use callbacks::*;

use crate::js_engine::gc::GarbageCollector;
use std::sync::{Arc, Mutex, Once};
use v8::{HandleScope, Local, TryCatch};

// Global V8 initialization state
static INIT_V8: Once = Once::new();
static DISPOSE_V8: Once = Once::new();
static V8_STATE: Mutex<V8State> = Mutex::new(V8State::Uninitialized);

#[derive(Debug, Clone, Copy, PartialEq)]
enum V8State {
    Uninitialized,
    Initialized,
    Disposed,
}

pub struct V8Runtime {
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    gc: Arc<Mutex<GarbageCollector>>,
}

impl V8Runtime {
    pub fn new() -> Result<Self, V8Error> {
        // Initialize V8 only once per process
        Self::ensure_v8_initialized()?;

        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        
        let context = {
            let scope = &mut v8::HandleScope::new(&mut isolate);
            let context = v8::Context::new(scope);
            v8::Global::new(scope, context)
        };

        let gc = Arc::new(Mutex::new(GarbageCollector::new()));

        Ok(Self {
            isolate,
            context,
            gc,
        })
    }

    fn ensure_v8_initialized() -> Result<(), V8Error> {
        let mut init_result = Ok(());
        
        INIT_V8.call_once(|| {
            match Self::initialize_v8() {
                Ok(_) => {
                    if let Ok(mut state) = V8_STATE.lock() {
                        *state = V8State::Initialized;
                    } else {
                        init_result = Err(V8Error::InitializationFailed);
                    }
                },
                Err(e) => {
                    init_result = Err(e);
                }
            }
        });

        // Check if initialization was successful
        if init_result.is_ok() {
            let state = V8_STATE.lock().map_err(|_| V8Error::InitializationFailed)?;
            if *state != V8State::Initialized {
                return Err(V8Error::InitializationFailed);
            }
        }

        init_result
    }

    fn initialize_v8() -> Result<(), V8Error> {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
        Ok(())
    }

    pub fn dispose_v8() {
        DISPOSE_V8.call_once(|| {
            if let Ok(mut state) = V8_STATE.lock() {
                if *state == V8State::Initialized {
                    unsafe {
                        v8::V8::dispose();
                    }
                    v8::V8::dispose_platform();
                    *state = V8State::Disposed;
                }
            }
        });
    }

    pub fn is_v8_initialized() -> bool {
        V8_STATE.lock()
            .map(|state| *state == V8State::Initialized)
            .unwrap_or(false)
    }

    fn with_context_scope<T, F>(&mut self, f: F) -> T
    where
        F: FnOnce(&mut v8::ContextScope<v8::HandleScope>) -> T,
    {
        let scope = &mut v8::HandleScope::new(&mut self.isolate);
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        f(scope)
    }

    pub fn execute(&mut self, source: &str) -> Result<serde_json::Value, V8Error> {
        self.with_context_scope(|scope| {
            let code = v8::String::new(scope, source)
                .ok_or(V8Error::InvalidSource)?;

            let mut try_catch = v8::TryCatch::new(scope);
            let script = v8::Script::compile(&mut try_catch, code, None)
                .ok_or_else(|| Self::extract_exception(&mut try_catch))?;

            let result = script.run(&mut try_catch)
                .ok_or_else(|| Self::extract_exception(&mut try_catch))?;

            Self::value_to_json(&mut try_catch, result)
        })
    }

    fn extract_exception(try_catch: &mut TryCatch<HandleScope>) -> V8Error {
        if let Some(exception) = try_catch.exception() {
            if let Some(message) = try_catch.message() {
                let msg_str = message.get(try_catch);
                return V8Error::ExecutionError(msg_str.to_rust_string_lossy(try_catch));
            }
            if let Some(exception_str) = exception.to_string(try_catch) {
                return V8Error::ExecutionError(exception_str.to_rust_string_lossy(try_catch));
            }
        }
        V8Error::ExecutionError("Unknown execution error".to_string())
    }

    fn value_to_json(
        scope: &mut HandleScope, 
        value: Local<v8::Value>
    ) -> Result<serde_json::Value, V8Error> {
        if value.is_null() || value.is_undefined() {
            Ok(serde_json::Value::Null)
        } else if value.is_boolean() {
            Ok(serde_json::Value::Bool(value.boolean_value(scope)))
        } else if value.is_number() {
            let num = value.number_value(scope).unwrap_or(0.0);
            Ok(serde_json::Value::Number(
                serde_json::Number::from_f64(num)
                    .unwrap_or_else(|| serde_json::Number::from(0))
            ))
        } else if value.is_string() {
            let string = value.to_string(scope).unwrap();
            Ok(serde_json::Value::String(string.to_rust_string_lossy(scope)))
        } else if value.is_array() {
            let array = v8::Local::<v8::Array>::try_from(value).unwrap();
            let mut result = Vec::with_capacity(array.length() as usize);
            for i in 0..array.length() {
                if let Some(element) = array.get_index(scope, i) {
                    result.push(Self::value_to_json(scope, element)?);
                }
            }
            Ok(serde_json::Value::Array(result))
        } else if value.is_object() {
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            let mut result = serde_json::Map::new();
            if let Some(property_names) = object.get_own_property_names(
                scope, 
                v8::GetPropertyNamesArgs::default()
            ) {
                for i in 0..property_names.length() {
                    if let Some(key) = property_names.get_index(scope, i) {
                        if let Some(key_str) = key.to_string(scope) {
                            let key_string = key_str.to_rust_string_lossy(scope);
                            if let Some(prop_value) = object.get(scope, key) {
                                result.insert(
                                    key_string, 
                                    Self::value_to_json(scope, prop_value)?
                                );
                            }
                        }
                    }
                }
            }
            Ok(serde_json::Value::Object(result))
        } else {
            Ok(serde_json::Value::Null)
        }
    }

    pub fn bind_function(&mut self, name: &str) -> Result<(), V8Error> {
        self.with_context_scope(|scope| {
            let function_name = v8::String::new(scope, name)
                .ok_or(V8Error::InvalidFunctionName)?;

            let function_template = v8::FunctionTemplate::new(
                scope,
                |scope: &mut HandleScope,
                 _args: v8::FunctionCallbackArguments,
                 mut rv: v8::ReturnValue| {
                    let undefined = v8::undefined(scope);
                    rv.set(undefined.into());
                }
            );

            let function = function_template
                .get_function(scope)
                .ok_or(V8Error::FunctionCreationFailed)?;

            let global = scope.get_current_context().global(scope);
            global
                .set(scope, function_name.into(), function.into())
                .ok_or(V8Error::BindingFailed)?;

            Ok(())
        })
    }

    pub fn bind_console_log(&mut self) -> Result<(), V8Error> {
        self.with_context_scope(|scope| {
            let console_obj = v8::Object::new(scope);
            let console_str = v8::String::new(scope, "console")
                .ok_or(V8Error::InvalidFunctionName)?;

            let log_template = v8::FunctionTemplate::new(
                scope,
                |scope: &mut HandleScope,
                 args: v8::FunctionCallbackArguments,
                 mut rv: v8::ReturnValue| {
                    for i in 0..args.length() {
                        let arg = args.get(i);
                        if let Some(str_val) = arg.to_string(scope) {
                            let rust_string = str_val.to_rust_string_lossy(scope);
                            println!("{}", rust_string);
                        }
                    }
                    let undefined = v8::undefined(scope);
                    rv.set(undefined.into());
                }
            );

            let log_function = log_template
                .get_function(scope)
                .ok_or(V8Error::FunctionCreationFailed)?;

            let log_str = v8::String::new(scope, "log")
                .ok_or(V8Error::InvalidFunctionName)?;

            console_obj.set(scope, log_str.into(), log_function.into())
                .ok_or(V8Error::BindingFailed)?;

            let global = scope.get_current_context().global(scope);
            global.set(scope, console_str.into(), console_obj.into())
                .ok_or(V8Error::BindingFailed)?;

            Ok(())
        })
    }

    pub fn bind_arithmetic_functions(&mut self) -> Result<(), V8Error> {
        self.with_context_scope(|scope| {
            let global = scope.get_current_context().global(scope);

            let add_template = v8::FunctionTemplate::new(
                scope,
                |scope: &mut HandleScope,
                 args: v8::FunctionCallbackArguments,
                 mut rv: v8::ReturnValue| {
                    if args.length() >= 2 {
                        let a = args.get(0).number_value(scope).unwrap_or(0.0);
                        let b = args.get(1).number_value(scope).unwrap_or(0.0);
                        let result = v8::Number::new(scope, a + b);
                        rv.set(result.into());
                    } else {
                        let undefined = v8::undefined(scope);
                        rv.set(undefined.into());
                    }
                }
            );

            let multiply_template = v8::FunctionTemplate::new(
                scope,
                |scope: &mut HandleScope,
                 args: v8::FunctionCallbackArguments,
                 mut rv: v8::ReturnValue| {
                    if args.length() >= 2 {
                        let a = args.get(0).number_value(scope).unwrap_or(0.0);
                        let b = args.get(1).number_value(scope).unwrap_or(0.0);
                        let result = v8::Number::new(scope, a * b);
                        rv.set(result.into());
                    } else {
                        let undefined = v8::undefined(scope);
                        rv.set(undefined.into());
                    }
                }
            );

            let add_function = add_template.get_function(scope)
                .ok_or(V8Error::FunctionCreationFailed)?;
            let multiply_function = multiply_template.get_function(scope)
                .ok_or(V8Error::FunctionCreationFailed)?;

            let add_name = v8::String::new(scope, "add")
                .ok_or(V8Error::InvalidFunctionName)?;
            let multiply_name = v8::String::new(scope, "multiply")
                .ok_or(V8Error::InvalidFunctionName)?;

            global.set(scope, add_name.into(), add_function.into())
                .ok_or(V8Error::BindingFailed)?;
            global.set(scope, multiply_name.into(), multiply_function.into())
                .ok_or(V8Error::BindingFailed)?;

            Ok(())
        })
    }

    pub fn create_object(&mut self) -> Result<v8::Global<v8::Object>, V8Error> {
        Ok(self.with_context_scope(|scope| {
            let object = v8::Object::new(scope);
            v8::Global::new(scope, object)
        }))
    }

    pub fn create_array(&mut self, length: u32) -> Result<v8::Global<v8::Array>, V8Error> {
        Ok(self.with_context_scope(|scope| {
            let array = v8::Array::new(scope, length as i32);
            v8::Global::new(scope, array)
        }))
    }

    pub fn create_string(&mut self, content: &str) -> Result<v8::Global<v8::String>, V8Error> {
        self.with_context_scope(|scope| {
            let string = v8::String::new(scope, content)
                .ok_or(V8Error::TypeConversionError)?;
            Ok(v8::Global::new(scope, string))
        })
    }

    pub fn create_number(&mut self, value: f64) -> v8::Global<v8::Number> {
        self.with_context_scope(|scope| {
            let number = v8::Number::new(scope, value);
            v8::Global::new(scope, number)
        })
    }

    pub fn create_boolean(&mut self, value: bool) -> v8::Global<v8::Boolean> {
        self.with_context_scope(|scope| {
            let boolean = v8::Boolean::new(scope, value);
            v8::Global::new(scope, boolean)
        })
    }

    pub fn force_gc(&mut self) {
        self.isolate.low_memory_notification();
    }

    pub fn trigger_gc(&self) -> Result<(), V8Error> {
        if let Ok(mut gc) = self.gc.try_lock() {
            gc.collect();
            Ok(())
        } else {
            Err(V8Error::GarbageCollectionFailed)
        }
    }

    pub fn heap_stats(&mut self) -> v8::HeapStatistics {
        let mut stats = v8::HeapStatistics::default();
        self.isolate.get_heap_statistics(&mut stats);
        stats
    }

    pub fn memory_usage(&mut self) -> usize {
        let stats = self.heap_stats();
        stats.used_heap_size()
    }
}

unsafe impl Send for V8Runtime {}
unsafe impl Sync for V8Runtime {}

impl Drop for V8Runtime {
    fn drop(&mut self) {
        self.force_gc();
        // Note: We don't dispose V8 here as it should only be disposed once per process
        // V8 disposal should happen at application shutdown via V8Runtime::dispose_v8()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum V8Error {
    #[error("Invalid source code")]
    InvalidSource,
    #[error("Execution error: {0}")]
    ExecutionError(String),
    #[error("Invalid function name")]
    InvalidFunctionName,
    #[error("Function creation failed")]
    FunctionCreationFailed,
    #[error("Binding failed")]
    BindingFailed,
    #[error("Type conversion error")]
    TypeConversionError,
    #[error("Garbage collection failed")]
    GarbageCollectionFailed,
    #[error("V8 initialization failed")]
    InitializationFailed,
}