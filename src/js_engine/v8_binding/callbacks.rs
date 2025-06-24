use crate::core::network::fetch::FetchEngine;
use crate::pwa::service_worker::ServiceWorkerRuntime;
use std::sync::Arc;
use tokio::sync::RwLock;
use v8::{HandleScope, FunctionCallbackArguments, ReturnValue, PromiseResolver, Object, Function, String as V8String};
use tracing::log;

pub struct WebApiBindings {
    fetch_engine: Arc<FetchEngine>,
    service_worker_runtime: Arc<RwLock<ServiceWorkerRuntime>>,
}

impl WebApiBindings {
    pub fn new() -> Self {
        Self {
            fetch_engine: Arc::new(FetchEngine::new()),
            service_worker_runtime: Arc::new(RwLock::new(ServiceWorkerRuntime::new())),
        }
    }

    pub fn console_log(scope: &mut HandleScope, args: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        let mut message = String::new();
        for i in 0..args.length() {
            if i > 0 {
                message.push(' ');
            }
            let arg = args.get(i);
            if let Some(string) = arg.to_string(scope) {
                message.push_str(&string.to_rust_string_lossy(scope));
            }
        }
        println!("[JS] {}", message);
        retval.set(v8::undefined(scope).into());
    }

    pub fn fetch(scope: &mut HandleScope, args: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        if args.length() == 0 {
            let exception = V8String::new(scope, "fetch requires at least one argument").unwrap();
            scope.throw_exception(exception.into());
            return;
        }
        let url_arg = args.get(0);
        let _url = if let Some(url_str) = url_arg.to_string(scope) {
            url_str.to_rust_string_lossy(scope)
        } else {
            let exception = V8String::new(scope, "URL must be a string").unwrap();
            scope.throw_exception(exception.into());
            return;
        };
        let promise_resolver = PromiseResolver::new(scope).unwrap();
        let promise = promise_resolver.get_promise(scope);
        retval.set(promise.into());
    }

    pub fn serial_request_port(scope: &mut HandleScope, _: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        let promise_resolver = PromiseResolver::new(scope).unwrap();
        let promise = promise_resolver.get_promise(scope);
        let port_object = Object::new(scope);

        let open_name = V8String::new(scope, "open").unwrap();
        let open_function = Function::new(
            scope,
            |scope: &mut HandleScope, args: FunctionCallbackArguments, mut retval: ReturnValue| {
                WebApiBindings::serial_port_open(scope, &args, &mut retval);
            },
        )
        .unwrap();
        port_object.set(scope, open_name.into(), open_function.into());

        let write_name = V8String::new(scope, "write").unwrap();
        let write_function = Function::new(
            scope,
            |scope: &mut HandleScope, args: FunctionCallbackArguments, mut retval: ReturnValue| {
                WebApiBindings::serial_port_write(scope, &args, &mut retval);
            },
        )
        .unwrap();
        port_object.set(scope, write_name.into(), write_function.into());

        promise_resolver.resolve(scope, port_object.into());
        retval.set(promise.into());
    }

    fn serial_port_open(scope: &mut HandleScope, _: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        let promise_resolver = PromiseResolver::new(scope).unwrap();
        let promise = {
            // Limit the borrow of scope to this block
            promise_resolver.get_promise(scope)
        };
        let undefined = v8::undefined(scope);
        promise_resolver.resolve(scope, undefined.into());
        retval.set(promise.into());
    }

    fn serial_port_write(scope: &mut HandleScope, args: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        if args.length() > 0 {
            let data = args.get(0);
            log::info!("Serial port write: {:?}", data);
        }
        let promise_resolver = PromiseResolver::new(scope).unwrap();
        let promise = promise_resolver.get_promise(scope);
        let undefined = v8::undefined(scope);
        promise_resolver.resolve(scope, undefined.into());
        retval.set(promise.into());
    }

    pub fn cache_open(scope: &mut HandleScope, args: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        if args.length() == 0 {
            let exception = V8String::new(scope, "Cache name required").unwrap();
            scope.throw_exception(exception.into());
            return;
        }
        let _cache_name = args.get(0).to_string(scope).unwrap().to_rust_string_lossy(scope);
        let promise_resolver = PromiseResolver::new(scope).unwrap();
        let promise = promise_resolver.get_promise(scope);
        let cache_object = Object::new(scope);

        let add_name = V8String::new(scope, "add").unwrap();
        let add_function = Function::new(
            scope,
            |scope: &mut HandleScope, args: FunctionCallbackArguments, mut retval: ReturnValue| {
                WebApiBindings::cache_add(scope, &args, &mut retval);
            },
        )
        .unwrap();
        cache_object.set(scope, add_name.into(), add_function.into());

        let match_name = V8String::new(scope, "match").unwrap();
        let match_function = Function::new(
            scope,
            |scope: &mut HandleScope, args: FunctionCallbackArguments, mut retval: ReturnValue| {
                WebApiBindings::cache_match(scope, &args, &mut retval);
            },
        )
        .unwrap();
        cache_object.set(scope, match_name.into(), match_function.into());

        promise_resolver.resolve(scope, cache_object.into());
        retval.set(promise.into());
    }

    fn cache_add(scope: &mut HandleScope, _: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        let promise_resolver = PromiseResolver::new(scope).unwrap();
        let promise = promise_resolver.get_promise(scope);
        let undefined = v8::undefined(scope);
        promise_resolver.resolve(scope, undefined.into());
        retval.set(promise.into());
    }

    fn cache_match(scope: &mut HandleScope, _: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        let promise_resolver = PromiseResolver::new(scope).unwrap();
        let promise = promise_resolver.get_promise(scope);
        let undefined = v8::undefined(scope);
        promise_resolver.resolve(scope, undefined.into());
        retval.set(promise.into());
    }

    pub fn register_service_worker(scope: &mut HandleScope, args: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        if args.length() == 0 {
            let exception = V8String::new(scope, "Service worker script URL required").unwrap();
            scope.throw_exception(exception.into());
            return;
        }
        let _script_url = args.get(0).to_string(scope).unwrap().to_rust_string_lossy(scope);
        let promise_resolver = PromiseResolver::new(scope).unwrap();
        let promise = promise_resolver.get_promise(scope);
        let registration_object = Object::new(scope);
        let active_name = V8String::new(scope, "active").unwrap();
        let worker_object = Object::new(scope);
        registration_object.set(scope, active_name.into(), worker_object.into());
        promise_resolver.resolve(scope, registration_object.into());
        retval.set(promise.into());
    }

    pub fn local_storage_get_item(scope: &mut HandleScope, args: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        if args.length() == 0 {
            retval.set(v8::null(scope).into());
            return;
        }
        let _key = args.get(0).to_string(scope).unwrap().to_rust_string_lossy(scope);
        let stored_value = V8String::new(scope, "stored_value").unwrap();
        retval.set(stored_value.into());
    }

    pub fn local_storage_set_item(scope: &mut HandleScope, args: &FunctionCallbackArguments, retval: &mut ReturnValue) {
        if args.length() < 2 {
            let exception = V8String::new(scope, "setItem requires key and value").unwrap();
            scope.throw_exception(exception.into());
            return;
        }
        let key = args.get(0).to_string(scope).unwrap().to_rust_string_lossy(scope);
        let value = args.get(1).to_string(scope).unwrap().to_rust_string_lossy(scope);
        log::info!("localStorage.setItem('{}', '{}')", key, value);
        retval.set(v8::undefined(scope).into());
    }
}

impl Default for WebApiBindings {
    fn default() -> Self {
        Self::new()
    }
}
