pub mod callbacks;

pub use callbacks::*;

use crate::js_engine::gc::GarbageCollector;
use std::sync::{Arc, Mutex};
use v8::{Context, HandleScope, Isolate, Local, TryCatch};

pub struct V8Runtime {
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    gc: Arc<Mutex<GarbageCollector>>,
}

impl V8Runtime {
    pub fn new() -> Result<Self, V8Error> {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();

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

    pub fn execute(&mut self, source: &str) -> Result<serde_json::Value, V8Error> {
        let scope = &mut v8::HandleScope::new(&mut self.isolate);
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);

        let code = v8::String::new(scope, source)
            .ok_or(V8Error::InvalidSource)?;

        let mut try_catch = v8::TryCatch::new(scope);
        let script = v8::Script::compile(&mut try_catch, code, None)
            .ok_or_else(|| self.get_exception_message(&mut try_catch))?;

        let result = script.run(&mut try_catch)
            .ok_or_else(|| self.get_exception_message(&mut try_catch))?;

        self.v8_value_to_json(&mut try_catch, result)
    }

    fn get_exception_message(&self, try_catch: &mut TryCatch<HandleScope>) -> V8Error {
        let exception = try_catch.exception().unwrap();
        let exception_string = exception.to_string(try_catch.scope()).unwrap();
        V8Error::ExecutionError(exception_string.to_rust_string_lossy(try_catch.scope()))
    }

    fn v8_value_to_json(&self, scope: &mut HandleScope, value: Local<v8::Value>) -> Result<serde_json::Value, V8Error> {
        if value.is_null() || value.is_undefined() {
            Ok(serde_json::Value::Null)
        } else if value.is_boolean() {
            Ok(serde_json::Value::Bool(value.boolean_value(scope)))
        } else if value.is_number() {
            Ok(serde_json::Value::Number(
                serde_json::Number::from_f64(value.number_value(scope).unwrap_or(0.0))
                    .unwrap_or(serde_json::Number::from(0))
            ))
        } else if value.is_string() {
            let string = value.to_string(scope).unwrap();
            Ok(serde_json::Value::String(string.to_rust_string_lossy(scope)))
        } else if value.is_array() {
            let array = v8::Local::<v8::Array>::try_from(value).unwrap();
            let mut result = Vec::new();
            for i in 0..array.length() {
                let element = array.get_index(scope, i).unwrap();
                result.push(self.v8_value_to_json(scope, element)?);
            }
            Ok(serde_json::Value::Array(result))
        } else if value.is_object() {
            let object = v8::Local::<v8::Object>::try_from(value).unwrap();
            let mut result = serde_json::Map::new();
            let property_names = object.get_own_property_names(scope, v8::GetPropertyNamesArgs::default()).unwrap();
            for i in 0..property_names.length() {
                let key = property_names.get_index(scope, i).unwrap();
                let key_string = key.to_string(scope).unwrap().to_rust_string_lossy(scope);
                let value = object.get(scope, key).unwrap();
                result.insert(key_string, self.v8_value_to_json(scope, value)?);
            }
            Ok(serde_json::Value::Object(result))
        } else {
            Ok(serde_json::Value::Null)
        }
    }

    pub fn bind_function<F>(&mut self, name: &str, callback: F) -> Result<(), V8Error>
    where
        F: Fn(&v8::FunctionCallbackArguments, &mut v8::ReturnValue) + 'static,
    {
        let scope = &mut v8::HandleScope::new(&mut self.isolate);
        let context = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, context);

        let function_name = v8::String::new(scope, name)
            .ok_or(V8Error::InvalidFunctionName)?;

        let function_template = v8::FunctionTemplate::new(scope, callback);
        let function = function_template.get_function(scope)
            .ok_or(V8Error::FunctionCreationFailed)?;

        let global = context.global(scope);
        global.set(scope, function_name.into(), function.into())
            .ok_or(V8Error::BindingFailed)?;

        Ok(())
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
}