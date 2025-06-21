# Deployment Guide

## System Requirements

### Minimum
- Vulkan 1.2 compatible GPU
- 4GB RAM
- 2GB disk space

### Recommended
- Vulkan 1.3 compatible GPU
- 8GB RAM
- 10GB disk space
- SSD storage

## Installation Methods

### Docker
```bash
docker run -p 8080:8080 vulkan-renderer:latest
```

### Kubernetes
```bash
kubectl apply -f k8s/
```

### Native Binary
```bash
cargo install --path .
vulkan-renderer --config config.toml
```

## Configuration

Create `config.toml`:
```toml
[renderer]
backend = "vulkan"
vsync = true
msaa_samples = 4

[security]
sandbox_level = "strict"
allow_file_access = false

[performance]
js_jit_threshold = 1000
gc_pressure_threshold = 0.8
```