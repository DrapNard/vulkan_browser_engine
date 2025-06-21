# API Reference

## Core APIs

### BrowserEngine
```rust
pub struct BrowserEngine {
    pub fn new() -> Result<Self, EngineError>
    pub fn load_url(&mut self, url: &str) -> Result<(), LoadError>
    pub fn execute_script(&mut self, script: &str) -> Result<JsValue, JsError>
    pub fn render_frame(&mut self) -> Result<(), RenderError>
}
```

### JavaScript Bindings
```rust
pub trait JsBinding {
    fn bind_to_context(&self, context: &mut V8Context) -> Result<(), BindError>;
}
```

### PWA Runtime
```rust
pub struct PwaRuntime {
    pub fn install_app(&mut self, manifest: &Manifest) -> Result<AppId, PwaError>
    pub fn register_service_worker(&mut self, script: &str) -> Result<WorkerId, PwaError>
}
```

## Web APIs

### Serial Port API
```javascript
const port = await navigator.serial.requestPort();
await port.open({ baudRate: 9600 });
const writer = port.writable.getWriter();
await writer.write(new TextEncoder().encode("Hello"));
```

### Cache API
```javascript
const cache = await caches.open('v1');
await cache.add('/offline.html');
const response = await cache.match('/offline.html');
```