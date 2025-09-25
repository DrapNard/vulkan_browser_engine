use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use v8::{
    Function, FunctionCallbackArguments, HandleScope, Local, Object, PromiseResolver, ReturnValue,
    String as V8String, TryCatch,
};

#[derive(thiserror::Error, Debug)]
pub enum CallbackError {
    #[error("V8 operation failed: {0}")]
    V8Error(String),
    #[error("Invalid argument at index {index}: {message}")]
    InvalidArgument { index: i32, message: String },
    #[error("Promise creation failed")]
    PromiseCreation,
    #[error("Function binding failed: {0}")]
    FunctionBinding(String),
}

type CallbackResult<T> = Result<T, CallbackError>;

pub struct CallbackRegistry {
    functions: Arc<RwLock<HashMap<String, v8::Global<Function>>>>,
}

impl Default for CallbackRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CallbackRegistry {
    pub fn new() -> Self {
        Self {
            functions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn register_function(&self, name: String, function: v8::Global<Function>) {
        let mut functions = self.functions.write().await;
        functions.insert(name, function);
    }

    pub async fn get_function(&self, name: &str) -> Option<v8::Global<Function>> {
        let functions = self.functions.read().await;
        functions.get(name).cloned()
    }
}

pub struct V8CallbackHelper;

impl V8CallbackHelper {
    pub fn extract_string_argument<'s>(
        scope: &mut HandleScope<'s>,
        args: &FunctionCallbackArguments,
        index: i32,
    ) -> CallbackResult<String> {
        if index >= args.length() {
            return Err(CallbackError::InvalidArgument {
                index,
                message: "Argument index out of bounds".to_string(),
            });
        }

        let arg = args.get(index);
        let v8_string = arg
            .to_string(scope)
            .ok_or_else(|| CallbackError::InvalidArgument {
                index,
                message: "Cannot convert to string".to_string(),
            })?;

        Ok(v8_string.to_rust_string_lossy(scope))
    }

    pub fn create_promise_with_resolver<'s>(
        scope: &mut HandleScope<'s>,
    ) -> CallbackResult<(Local<'s, v8::Promise>, Local<'s, PromiseResolver>)> {
        let resolver = PromiseResolver::new(scope).ok_or(CallbackError::PromiseCreation)?;

        let promise = resolver.get_promise(scope);
        Ok((promise, resolver))
    }

    pub fn create_v8_string<'s>(
        scope: &mut HandleScope<'s>,
        content: &str,
    ) -> CallbackResult<Local<'s, V8String>> {
        V8String::new(scope, content).ok_or_else(|| {
            CallbackError::V8Error(format!("Failed to create V8 string: {}", content))
        })
    }

    pub fn throw_error<'s>(scope: &mut HandleScope<'s>, message: &str) {
        if let Ok(error_string) = Self::create_v8_string(scope, message) {
            scope.throw_exception(error_string.into());
        }
    }

    pub fn set_undefined_return<'s>(scope: &mut HandleScope<'s>, retval: &mut ReturnValue) {
        retval.set(v8::undefined(scope).into());
    }

    pub fn set_null_return<'s>(scope: &mut HandleScope<'s>, retval: &mut ReturnValue) {
        retval.set(v8::null(scope).into());
    }

    pub fn create_empty_object<'s>(scope: &mut HandleScope<'s>) -> Local<'s, Object> {
        Object::new(scope)
    }

    pub fn bind_method_to_object<'s>(
        scope: &mut HandleScope<'s>,
        object: Local<'s, Object>,
        method_name: &str,
        callback: impl v8::MapFnTo<v8::FunctionCallback>,
    ) -> CallbackResult<()> {
        let name = Self::create_v8_string(scope, method_name)?;
        let function = Function::new(scope, callback)
            .ok_or_else(|| CallbackError::FunctionBinding(method_name.to_string()))?;

        object.set(scope, name.into(), function.into());
        Ok(())
    }

    pub fn handle_with_try_catch<F, R>(scope: &mut HandleScope, operation: F) -> CallbackResult<R>
    where
        F: FnOnce(&mut HandleScope) -> Option<R>,
    {
        let mut try_catch = TryCatch::new(scope);
        let result = operation(&mut try_catch);

        if try_catch.has_caught() {
            if let Some(exception) = try_catch.exception() {
                if let Some(message) = exception.to_string(&mut try_catch) {
                    let error_msg = message.to_rust_string_lossy(&mut try_catch);
                    return Err(CallbackError::V8Error(error_msg));
                }
            }
            return Err(CallbackError::V8Error("Unknown V8 exception".to_string()));
        }

        result.ok_or_else(|| CallbackError::V8Error("Operation failed".to_string()))
    }
}

pub struct ConsoleCallbacks;

impl ConsoleCallbacks {
    pub fn log(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let mut message_parts = Vec::with_capacity(args.length() as usize);

        for i in 0..args.length() {
            match V8CallbackHelper::extract_string_argument(scope, &args, i) {
                Ok(part) => message_parts.push(part),
                Err(_) => message_parts.push("[object]".to_string()),
            }
        }

        let message = message_parts.join(" ");
        info!("[Console] {}", message);
        V8CallbackHelper::set_undefined_return(scope, &mut retval);
    }

    pub fn warn(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let mut message_parts = Vec::with_capacity(args.length() as usize);

        for i in 0..args.length() {
            match V8CallbackHelper::extract_string_argument(scope, &args, i) {
                Ok(part) => message_parts.push(part),
                Err(_) => message_parts.push("[object]".to_string()),
            }
        }

        let message = message_parts.join(" ");
        warn!("[Console] {}", message);
        V8CallbackHelper::set_undefined_return(scope, &mut retval);
    }

    pub fn error(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let mut message_parts = Vec::with_capacity(args.length() as usize);

        for i in 0..args.length() {
            match V8CallbackHelper::extract_string_argument(scope, &args, i) {
                Ok(part) => message_parts.push(part),
                Err(_) => message_parts.push("[object]".to_string()),
            }
        }

        let message = message_parts.join(" ");
        error!("[Console] {}", message);
        V8CallbackHelper::set_undefined_return(scope, &mut retval);
    }
}

pub struct FetchCallbacks;

impl FetchCallbacks {
    pub fn fetch(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let url = match V8CallbackHelper::extract_string_argument(scope, &args, 0) {
            Ok(url) => url,
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "fetch requires a URL argument");
                return;
            }
        };

        let (promise, resolver) = match V8CallbackHelper::create_promise_with_resolver(scope) {
            Ok((p, r)) => (p, r),
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "Failed to create promise");
                return;
            }
        };

        debug!("Fetch request initiated for URL: {}", url);

        let response_object = V8CallbackHelper::create_empty_object(scope);

        if let Ok(status_key) = V8CallbackHelper::create_v8_string(scope, "status") {
            let status_value = v8::Number::new(scope, 200.0);
            response_object.set(scope, status_key.into(), status_value.into());
        }

        if let Ok(ok_key) = V8CallbackHelper::create_v8_string(scope, "ok") {
            let ok_value = v8::Boolean::new(scope, true);
            response_object.set(scope, ok_key.into(), ok_value.into());
        }

        resolver.resolve(scope, response_object.into());
        retval.set(promise.into());
    }
}

pub struct StorageCallbacks;

impl StorageCallbacks {
    pub fn get_item(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let _key = match V8CallbackHelper::extract_string_argument(scope, &args, 0) {
            Ok(key) => key,
            Err(_) => {
                V8CallbackHelper::set_null_return(scope, &mut retval);
                return;
            }
        };

        match V8CallbackHelper::create_v8_string(scope, "mock_stored_value") {
            Ok(value) => retval.set(value.into()),
            Err(_) => V8CallbackHelper::set_null_return(scope, &mut retval),
        }
    }

    pub fn set_item(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let key = match V8CallbackHelper::extract_string_argument(scope, &args, 0) {
            Ok(key) => key,
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "setItem requires a key argument");
                return;
            }
        };

        let value = match V8CallbackHelper::extract_string_argument(scope, &args, 1) {
            Ok(value) => value,
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "setItem requires a value argument");
                return;
            }
        };

        info!("localStorage.setItem('{}', '{}')", key, value);
        V8CallbackHelper::set_undefined_return(scope, &mut retval);
    }

    pub fn remove_item(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let key = match V8CallbackHelper::extract_string_argument(scope, &args, 0) {
            Ok(key) => key,
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "removeItem requires a key argument");
                return;
            }
        };

        info!("localStorage.removeItem('{}')", key);
        V8CallbackHelper::set_undefined_return(scope, &mut retval);
    }

    pub fn clear(
        scope: &mut v8::HandleScope,
        _args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        info!("localStorage.clear()");
        V8CallbackHelper::set_undefined_return(scope, &mut retval);
    }
}

pub struct SerialCallbacks;

impl SerialCallbacks {
    pub fn request_port(
        scope: &mut v8::HandleScope,
        _args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let (promise, resolver) = match V8CallbackHelper::create_promise_with_resolver(scope) {
            Ok((p, r)) => (p, r),
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "Failed to create promise");
                return;
            }
        };

        let port_object = V8CallbackHelper::create_empty_object(scope);

        if let Err(e) =
            V8CallbackHelper::bind_method_to_object(scope, port_object, "open", Self::port_open)
        {
            error!("Failed to bind port.open: {}", e);
        }

        if let Err(e) =
            V8CallbackHelper::bind_method_to_object(scope, port_object, "write", Self::port_write)
        {
            error!("Failed to bind port.write: {}", e);
        }

        if let Err(e) =
            V8CallbackHelper::bind_method_to_object(scope, port_object, "close", Self::port_close)
        {
            error!("Failed to bind port.close: {}", e);
        }

        debug!("Serial port object created");
        resolver.resolve(scope, port_object.into());
        retval.set(promise.into());
    }

    pub fn port_open(
        scope: &mut v8::HandleScope,
        _args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let (promise, resolver) = match V8CallbackHelper::create_promise_with_resolver(scope) {
            Ok((p, r)) => (p, r),
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "Failed to create promise");
                return;
            }
        };

        info!("Serial port opened");
        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
        retval.set(promise.into());
    }

    pub fn port_write(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let (promise, resolver) = match V8CallbackHelper::create_promise_with_resolver(scope) {
            Ok((p, r)) => (p, r),
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "Failed to create promise");
                return;
            }
        };

        if let Ok(data) = V8CallbackHelper::extract_string_argument(scope, &args, 0) {
            info!("Serial port write: {}", data);
        }

        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
        retval.set(promise.into());
    }

    pub fn port_close(
        scope: &mut v8::HandleScope,
        _args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let (promise, resolver) = match V8CallbackHelper::create_promise_with_resolver(scope) {
            Ok((p, r)) => (p, r),
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "Failed to create promise");
                return;
            }
        };

        info!("Serial port closed");
        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
        retval.set(promise.into());
    }
}

pub struct TimerCallbacks;

impl TimerCallbacks {
    pub fn set_timeout(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        if args.length() < 2 {
            V8CallbackHelper::throw_error(scope, "setTimeout requires callback and delay");
            return;
        }

        let timer_id = v8::Number::new(scope, 1.0);
        debug!("setTimeout called");
        retval.set(timer_id.into());
    }

    pub fn set_interval(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        if args.length() < 2 {
            V8CallbackHelper::throw_error(scope, "setInterval requires callback and delay");
            return;
        }

        let timer_id = v8::Number::new(scope, 2.0);
        debug!("setInterval called");
        retval.set(timer_id.into());
    }

    pub fn clear_timeout(
        scope: &mut v8::HandleScope,
        _args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        debug!("clearTimeout called");
        V8CallbackHelper::set_undefined_return(scope, &mut retval);
    }

    pub fn clear_interval(
        scope: &mut v8::HandleScope,
        _args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        debug!("clearInterval called");
        V8CallbackHelper::set_undefined_return(scope, &mut retval);
    }
}

pub struct CacheCallbacks;

impl CacheCallbacks {
    pub fn open(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let cache_name = match V8CallbackHelper::extract_string_argument(scope, &args, 0) {
            Ok(name) => name,
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "Cache name required");
                return;
            }
        };

        let (promise, resolver) = match V8CallbackHelper::create_promise_with_resolver(scope) {
            Ok((p, r)) => (p, r),
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "Failed to create promise");
                return;
            }
        };

        let cache_object = V8CallbackHelper::create_empty_object(scope);

        if let Err(e) =
            V8CallbackHelper::bind_method_to_object(scope, cache_object, "add", Self::cache_add)
        {
            error!("Failed to bind cache.add: {}", e);
        }

        if let Err(e) =
            V8CallbackHelper::bind_method_to_object(scope, cache_object, "match", Self::cache_match)
        {
            error!("Failed to bind cache.match: {}", e);
        }

        debug!("Cache opened: {}", cache_name);
        resolver.resolve(scope, cache_object.into());
        retval.set(promise.into());
    }

    pub fn cache_add(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let (promise, resolver) = match V8CallbackHelper::create_promise_with_resolver(scope) {
            Ok((p, r)) => (p, r),
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "Failed to create promise");
                return;
            }
        };

        if let Ok(url) = V8CallbackHelper::extract_string_argument(scope, &args, 0) {
            debug!("Cache add: {}", url);
        }

        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
        retval.set(promise.into());
    }

    pub fn cache_match(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        mut retval: v8::ReturnValue,
    ) {
        let (promise, resolver) = match V8CallbackHelper::create_promise_with_resolver(scope) {
            Ok((p, r)) => (p, r),
            Err(_) => {
                V8CallbackHelper::throw_error(scope, "Failed to create promise");
                return;
            }
        };

        if let Ok(url) = V8CallbackHelper::extract_string_argument(scope, &args, 0) {
            debug!("Cache match: {}", url);
        }

        let undefined = v8::undefined(scope);
        resolver.resolve(scope, undefined.into());
        retval.set(promise.into());
    }
}
